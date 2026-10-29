use crate::{
    parsing::ast::{AstIds, AstValue, BinOp, UnaryOp}, typing::{EnumId, TypeId}, vm_compiling::{NumMode, VmFormatArgDisplayFn},
};


#[derive(Debug, Clone, PartialEq)]
pub struct UirFunction {
    pub id: FunctionId,
    pub name: String,
    pub locals: Vec<LocalDecl>,
    pub param_count: usize,
    pub return_type: TypeId,
    pub blocks: Vec<BasicBlock>,
    pub entry_block: BlockId,
}

impl UirFunction {
    #[must_use]
    pub fn new(id: FunctionId, name: String, param_count: usize, return_type: TypeId) -> Self {
        Self {
            id, name, param_count, return_type,
            locals: vec![
                LocalDecl {
                    id: LocalId::RETURN_PLACE,
                    type_id: return_type,
                    debug_name: Some("_0".to_string()),
                    is_mut: true,
                }
            ],
            blocks: vec![BasicBlock::new(Terminator::Unreachable)],
            entry_block: BlockId(0),
        }
    }

    pub fn new_local(&mut self, type_id: TypeId, debug_name: Option<String>, is_mut: bool) -> LocalId {
        let id = LocalId(self.locals.len().try_into().unwrap());
        self.locals.push(LocalDecl { id, type_id, debug_name, is_mut });
        id
    }

    pub fn new_temp(&mut self, type_id: TypeId) -> LocalId {
        self.new_local(type_id, None, false)
    }

    pub fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len().try_into().unwrap());
        self.blocks.push(BasicBlock::new(Terminator::Unreachable));
        id
    }

    pub fn add_statement(&mut self, block: BlockId, stmt: Statement) {
        self.blocks[block.0 as usize].statements.push(stmt);
    }

    pub fn set_terminator(&mut self, block: BlockId, term: Terminator) {
        self.blocks[block.0 as usize].terminator = term;
    }
}


#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct FunctionId(pub AstIds);
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct BlockId(pub AstIds);
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct LocalId(pub AstIds);

impl LocalId {
    /// `_0` is always the return place
    pub const RETURN_PLACE: Self = Self(0);
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalDecl {
    pub id: LocalId,
    pub type_id: TypeId,
    pub debug_name: Option<String>,
    pub is_mut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Place {
    pub local: LocalId,
    pub projections: Vec<Projection>,
}

impl Place {
    #[must_use] pub const fn from_local(local: LocalId) -> Self {
        Self { local, projections: Vec::new() }
    }

    #[must_use] pub fn field(mut self, index: usize) -> Self {
        self.projections.push(Projection::Field(index));
        self
    }

    #[must_use] pub fn deref(mut self) -> Self {
        self.projections.push(Projection::Deref);
        self
    }

    #[must_use] pub fn index(mut self, local: LocalId) -> Self {
        self.projections.push(Projection::Index(local));
        self
    }

    #[must_use] pub fn downcast(mut self, variant: usize) -> Self {
        self.projections.push(Projection::Downcast(variant));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Projection {
    /// `*p`
    Deref,
    /// `val.0` or `val.field`
    Field(usize),
    /// `slice[index_local]`
    Index(LocalId),
    /// `arr[const_index]`
    ConstantIndex(usize),
    /// Enum variant payload access
    Downcast(usize),
}


#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Void,
    Type(TypeId),
    Function(FunctionId),
    NativeFn(String),
}
impl Constant {
    pub fn from_val(val: &AstValue) -> Self {
        match val {
            AstValue::NumInt(x) => Self::Int(*x),
            AstValue::NumFloat(x) => Self::Float(*x),
            AstValue::Str(x) => Self::Str(x.clone()),
            AstValue::Bool(x) => Self::Bool(*x),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    Constant(Constant),
}

impl Operand {
    #[must_use] pub const fn copy(local: LocalId) -> Self {
        Self::Copy(Place::from_local(local))
    }

    #[must_use] pub const fn copy_place(place: Place) -> Self {
        Self::Copy(place)
    }

    #[must_use] pub const fn move_place(place: Place) -> Self {
        Self::Move(place)
    }

    #[must_use] pub const fn const_int(val: i64) -> Self {
        Self::Constant(Constant::Int(val))
    }

    #[must_use] pub const fn const_float(val: f64) -> Self {
        Self::Constant(Constant::Float(val))
    }

    #[must_use] pub const fn const_bool(val: bool) -> Self {
        Self::Constant(Constant::Bool(val))
    }

    #[must_use] pub const fn const_void() -> Self {
        Self::Constant(Constant::Void)
    }
}



#[derive(Debug, Clone, PartialEq)]
pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

impl BasicBlock {
    #[must_use]
    pub const fn new(terminator: Terminator) -> Self {
        Self { statements: Vec::new(), terminator }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Assign(Place, RValue),
    SetDiscriminant {
        place: Place,
        variant_index: usize,
    },
    StorageLive(LocalId),
    StorageDead(LocalId),
    Nop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RValue {
    Use(Operand),
    UnaryOp(UnaryOp, NumMode, Operand),
    BinaryOp(BinOp, NumMode, Operand, Operand),
    MakeRef { mutable: bool, place: Place },
    UseDiscriminant(Place),
    MakeAggregate(AggregateKind, Vec<Operand>),
    MakeSlice { ptr: Operand, len: Operand },
    GetLen(Place),
    Cast { operand: Operand, target_type: TypeId },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AggregateKind {
    Tuple,
    Array,
    Enum {
        enum_id: EnumId,
        variant_index: usize,
    },
    FormatString {
        display_fns: Vec<VmFormatArgDisplayFn>,
        str_pieces: Vec<String>,
    },
}


#[derive(Debug, Clone, PartialEq)]
pub enum Terminator {
    Goto(BlockId),
    MatchOnInt {
        discr: Operand,
        targets: Vec<(i64, BlockId)>,
        otherwise: BlockId,
    },
    Call {
        func: Operand,
        args: Vec<Operand>,
        destination: Place,
        target: Option<BlockId>,
    },
    /// returns value in `LocalId::RETURN_PLACE`
    Return,
    Panic { msg: Operand },
    Unreachable,
}