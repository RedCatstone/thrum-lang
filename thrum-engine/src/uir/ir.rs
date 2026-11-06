use crate::{
    parsing::ast::{AstIds, AstValue, BinOp, UnaryOp},typing::{EnumId, TypeId}, vm_compiling::NumMode,
};


#[derive(Debug, Clone, PartialEq)]
pub struct UirFunction {
    pub id: FunctionId,
    pub name: String,
    pub locals: Vec<LocalDecl>,
    pub param_count: usize,
    pub return_type: TypeId,
    pub frame_size: usize,
    pub blocks: Vec<BasicBlock>,
    pub entry_block: BlockId,
}

impl UirFunction {
    #[must_use]
    pub fn new(id: FunctionId, name: String, param_count: usize, return_type: TypeId, return_size: usize) -> Self {
        Self {
            id, name, param_count, return_type,
            locals: vec![
                LocalDecl {
                    id: LocalId::RETURN_PLACE,
                    type_id: return_type,
                    offset: 0,
                    size: return_size,
                    debug_name: Some("_0".into()),
                    is_mut: true,
                }
            ],
            frame_size: return_size,
            blocks: vec![BasicBlock::new("entry")],
            entry_block: BlockId(0),
        }
    }

    pub fn new_local(&mut self, type_id: TypeId, size: usize, debug_name: Option<Box<str>>, is_mut: bool) -> LocalId {
        let id = LocalId(self.locals.len().try_into().unwrap());
        let offset = self.frame_size;
        self.frame_size += size;
        self.locals.push(LocalDecl { id, type_id, offset, size, debug_name, is_mut });
        id
    }

    pub fn new_temp(&mut self, type_id: TypeId, size: usize) -> LocalId {
        self.new_local(type_id, size, None, false)
    }

    pub fn new_block(&mut self, debug_name: &'static str) -> BlockId {
        let id = BlockId(self.blocks.len().try_into().unwrap());
        self.blocks.push(BasicBlock::new(debug_name));
        id
    }

    pub fn add_statement(&mut self, block: BlockId, stmt: Statement) {
        self.blocks[block.0 as usize].statements.push(stmt);
    }

    /// only sets a terminator, if there wasn't one yet.
    pub fn set_terminator(&mut self, block: BlockId, term: Terminator) {
        let curr_term = &mut self.blocks[block.0 as usize].terminator;
        if *curr_term == Terminator::Unreachable {
           *curr_term = term;
        }
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
    pub offset: usize,
    pub size: usize,
    pub debug_name: Option<Box<str>>,
    pub is_mut: bool,
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
    #[must_use] pub fn from_val(val: &AstValue) -> Self {
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
    Copy(LocalId),
    Move(LocalId),
    Constant(Constant),
}



#[derive(Debug, Clone, PartialEq)]
pub struct BasicBlock {
    pub debug_name: &'static str,
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

impl BasicBlock {
    #[must_use]
    pub const fn new(debug_name: &'static str) -> Self {
        Self { statements: Vec::new(), terminator: Terminator::Unreachable, debug_name }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Assign(LocalId, RValue),
    /// `*pointer = value`
    Store { pointer: LocalId, value: Operand },
    SetDiscriminant {
        local: LocalId,
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
    MakeRef { mutable: bool, local: LocalId },
    Deref(LocalId),

    // Tuples
    MakeTuple(Vec<Operand>),
    MakeTupleArr { elem: Operand, length: usize },
    TupleGet { tuple: LocalId, offset: usize },
    TupleGetRef { tuple: LocalId, offset: usize },
    IndexRef { base: LocalId, index: Operand, elem_size: usize, len: usize },

    // Enums
    MakeEnum { enum_id: EnumId, variant_index: usize, payload: Operand },
    ReadDiscriminant(LocalId),
    SetDiscriminant(LocalId, Operand),

    MakeSlice { ptr: Operand, len: Operand },
    GetLen(LocalId),
    Cast { operand: Operand, target_type: TypeId },
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
        destination: LocalId,
        target: Option<BlockId>,
    },
    /// returns value in `LocalId::RETURN_PLACE`
    Return,
    Panic { msg: Operand },
    Unreachable,
}