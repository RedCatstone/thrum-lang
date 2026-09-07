use std::{collections::{HashMap, HashSet}, fmt::{self, Write}};

use crate::{
    ErrType, ProgramError, ProgramErrorData, WarnType, lexing::tokens::Span, nativelib::get_native_lib, parsing::ast::{AstArena, AstIds, ExprId, PatternId}, typing::{check_expressions::CheckExprCtx, type_vars::{SnapshotVarsState, TypeVar, TypeVarIsUsed, TypeVarScope}}, vm_compiling::{FunctionRegistry, NumMode, VmValue}
};

pub mod type_vars;
mod check_expressions;
mod check_patterns;
mod exhaustiveness;
mod coercion;


#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub enum Type {
    NumInt,
    NumFloat,
    Str,
    Bool,

    /// 0-bit type
    Void,
    /// something that never happens at runtime
    /// e.g. the type of (return, break, continue)-exprs
    Never,

    /// `const X = bool`, X has type `MetaType`
    MetaType,

    Enum(EnumId),
    EnumVariant { inner: TypeId, variant: usize },

    CustomType(CustomTypeId, TypeId),

    Tup(Vec<TypeTuple>),
    TupArr(TypeId, usize),

    Fn { param_types: Vec<TypeId>, return_type: TypeId },

    Borrow { inner: TypeId, mutable: bool, borrows_var: Option<TypeVarId> },

    Infer(TypeInferId),
    Error,
}


#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct TypeTuple {
    pub label: String,
    pub typ: TypeId,
}

#[derive(Debug)]
pub struct CustomType<'a> {
    name: Box<str>,
    impls: TypeVarScope<'a>
}


#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, PartialOrd)]
pub struct TypeId(pub AstIds);
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct EnumId(pub AstIds);
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq, PartialOrd)]
pub struct CustomTypeId(pub AstIds);
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct TypeInferId(pub AstIds);
#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct TypeVarId(pub AstIds);

impl TypeId {
    pub const ERROR: Self = Self(0);
    pub const NEVER: Self = Self(1);
    pub const VOID:  Self = Self(2);
    pub const TYPE:  Self = Self(3);
    pub const INT:   Self = Self(4);
    pub const FLOAT: Self = Self(5);
    pub const BOOL:  Self = Self(6);
    pub const STR:   Self = Self(7);

    pub const MAP_CONSTS: [(Self, Type); 8] = [
        (Self::ERROR, Type::Error), (Self::NEVER, Type::Never), (Self::VOID, Type::Void), (Self::TYPE, Type::MetaType),
        (Self::INT, Type::NumInt), (Self::FLOAT, Type::NumFloat), (Self::BOOL, Type::Bool), (Self::STR, Type::Str)
    ];
}



pub struct TypeChecker<'a> {
    error_data: &'a mut ProgramErrorData,
    ast: &'a AstArena,  // needs to be mut for auto-deref
    typed_ast: TypedAst,

    // maps variable names to their Id
    var_scopes: Vec<TypeVarScope<'a>>,

    type_arena: TypeArena,

    // if inference_types[0] is Some(TypeId(3))
    // => InferType 0 was resolved to TypeId 3
    inference_types: Vec<Option<TypeId>>,  // indexed with TypeInferId

    // implemented stuff on types
    // e.g. `impl Number { ... }`
    custom_types: Vec<CustomType<'a>>,  // indexed with CustomTypeId

    // for return
    curr_function_return_type: Option<TypeId>,
    // for break/continue
    curr_label_infos: Vec<LabelInfo<'a>>,
    // for impl so they can use Self and self
    curr_impl_self: Option<TypeId>,

    // meta compiling stuff, if a function gets compiled during the typechecking phase,
    // it gets kept and doesn't need to be compiled again in the VmCompiler stage
    compiled_functions: FunctionRegistry,
}

#[derive(Debug, Default)]
/// this struct gets build in the typechecker phase, and is read-only afterwards.
///
/// it has a bunch of notes telling the `VmCompiler` everything it needs to compile.
/// it needs to know to compile the ast-nodes (which are read-only after parsing already)
pub struct TypedAst {
    pub expr_types: Vec<TypeId>,  // Indexed with ExprId

    pub enum_defs: Vec<EnumDefinition>,  // indexed with EnumId

    pub vars: Vec<TypeVar>,  // indexed with TypeVarId
    pub resolved_expr_var: HashMap<ExprId, TypeVarId>,
    pub resolved_pattern_var: HashMap<PatternId, TypeVarId>,

    pub resolved_impl_self_type: HashMap<ExprId, TypeId>,
    pub resolved_type_instantian: HashMap<ExprId, ResolvedTypeInstantiation>,
    pub resolved_type_destruction_not_a_tuple: HashSet<PatternId>,
    pub resolved_num_mode: HashMap<ExprId, NumMode>,

    pub resolved_tuple_arr_length: HashMap<ExprId, usize>,
    pub resolved_tuple_type_coerce: HashMap<ExprId, Box<[String]>>,
    pub resolved_enum_variant: HashMap<ExprId, (EnumId, usize)>,
    pub resolved_enum_variant_pattern: HashMap<PatternId, (EnumId, usize)>,
    pub resolved_closure_fn_id: HashMap<ExprId, usize>,
    pub resolved_member_access: HashMap<ExprId, ResolvedMemberAccess>,
    pub resolved_labels: HashMap<ExprId, ExprId>,  // maps a labeled-expr to where it needs to jump to
    pub auto_derefs: HashMap<ExprId, usize>,  // amount of autoderefs
    pub move_expr: HashSet<ExprId>,
}
impl TypedAst {
    #[must_use]
    pub fn new(ast_exprs: usize) -> Self {
        Self { expr_types: vec![TypeId::ERROR; ast_exprs], ..Default::default() }
    }
    #[must_use] pub fn get_expr_type(&self, id: ExprId) -> TypeId { self.expr_types[id.0 as usize] }

    #[must_use] pub fn get_var(&self, id: TypeVarId) -> &TypeVar { &self.vars[id.0 as usize] }
    #[must_use] pub fn get_var_mut(&mut self, id: TypeVarId) -> &mut TypeVar { &mut self.vars[id.0 as usize] }
}

#[derive(Debug)]
pub struct TypeArena {
    // indexed with TypeId
    pub types: Vec<Type>,

    // duplicate types should get the same id
    // because of inference types there can still be dupes
    // but those get filtered out in the zonking phase.
    type_dedup: HashMap<Type, TypeId>,
}
impl TypeArena {
    #[must_use]
    fn new() -> Self {
        let mut ta = Self { types: Vec::new(), type_dedup: HashMap::new() };
        // add the hardcoded types
        for (id, typ) in TypeId::MAP_CONSTS {
            assert_eq!(id, ta.add_type(typ));
        }
        ta
    }

    #[must_use]
    pub fn get_type(&self, id: TypeId) -> Type { self.types[id.0 as usize].clone() }

    #[must_use]
    pub fn add_type(&mut self, typ: Type) -> TypeId {
        if let Some(&id) = self.type_dedup.get(&typ) {
            id
        } else {
            let id = TypeId(self.types.len().try_into().unwrap());
            self.types.push(typ.clone());
            self.type_dedup.insert(typ, id);
            id
        }
    }
}

#[derive(Debug)]
pub struct EnumDefinition {
    /// maps variant names to their `attached_tuple`
    /// e.g. "Some" -> `Type::Tup(...)`
    /// e.g. "None" -> `Type::Void`
    variants: Vec<(Box<str>, TypeId)>
}

pub struct LabelInfo<'ast> {
    label: &'ast str,
    expr: ExprId,
    typ: TypeId,
    break_snapshots: Vec<Option<SnapshotVarsState>>,
}

#[derive(Debug)]
pub enum ResolvedMemberAccess {
    TupleRefIndex { index: usize },  // tup.0
    TupleIndex { index: usize },  // tup^.0
    Member { constant: VmValue },  // Point.distance(p)
    MemberWithSelfSugar { constant: VmValue, self_sugar_expr: ExprId },  // p.distance()
    EnumWithNoData { i: usize }  // Option.None
}

#[derive(Debug)]
pub enum ResolvedTypeInstantiation {
    NewType,
    Tuple,
    EnumVariant(usize),
}

#[derive(Clone, Copy)]
pub enum UnifyMode {
    /// Used for Assignments / Function Args.
    /// e.g. `Option - Option.Some` works
    /// e.g. `Option.Some - Option` doesnt work
    Subtype,

    /// e.g. `Option.Some - Option` -> `Option`
    FindParentType,
}

impl TypeChecker<'_> {
    pub fn start(error_data: &mut ProgramErrorData, ast: &AstArena) -> (TypedAst, FunctionRegistry) {
        let mut tc = TypeChecker {
            error_data, ast,
            typed_ast: TypedAst::new(ast.exprs.len()),
            var_scopes: vec![TypeVarScope::default()],
            type_arena: TypeArena::new(),
            inference_types: Vec::new(),
            custom_types: Vec::new(),
            curr_function_return_type: None,
            curr_label_infos: Vec::new(),
            curr_impl_self: None,
            compiled_functions: FunctionRegistry::new(),
        };
        let native_lib = get_native_lib(&mut tc.type_arena);
        tc.load_prelude_from_lib(&native_lib);

        // check the main expression
        tc.check_expression(ExprId(0), &mut false, CheckExprCtx::default());

        tc.finalize_types_and_vars();

        (tc.typed_ast, tc.compiled_functions)
    }

    #[track_caller]
    fn error(&mut self, err_type: ErrType, span: Span) -> TypeId {
        self.error_data.errors.push(ProgramError {
            span,
            err_type,
            compiler_location: std::panic::Location::caller()
        });
        TypeId::ERROR
    }

    #[track_caller]
    fn type_mismatch(&mut self, expected: TypeId, found: TypeId, span: Span) -> TypeId {
        self.error(ErrType::TyperMismatch {
            expected: self.fmt_type(expected),
            found: self.fmt_type(found)
        }, span)
    }

    pub fn new_infer_type(&mut self) -> TypeId {
        let id = TypeInferId(self.inference_types.len().try_into().unwrap());
        self.inference_types.push(None);  // unresolved initially
        self.type_arena.add_type(Type::Infer(id))
    }

    #[must_use]
    pub fn prune_id_once(&self, mut id: TypeId) -> TypeId {
        while let Type::Infer(infer_id) = self.type_arena.types[id.0 as usize]
        && let Some(resolved_id) = self.inference_types[infer_id.0 as usize] {
            id = resolved_id;
        }
        id
    }
    #[must_use]
    pub fn prune_type_once(&self, id: TypeId) -> Type {
        let id = self.prune_id_once(id);
        self.type_arena.get_type(id)
    }
    #[must_use]
    pub fn prune_type_once_infer_err(&mut self, id: TypeId, err_span: Span) -> Type {
        let typ = self.prune_type_once(id);
        if let Type::Infer(_) = typ {
            self.error(ErrType::TyperTypeMustBeKnownHere { typ: self.fmt_type(id) }, err_span);
            Type::Error
        } else {
            typ
        }
    }

    #[track_caller]
    pub fn unify_types(&mut self, expected: TypeId, other: TypeId, span: Span, mode: UnifyMode) -> TypeId {
        let mut mismatch = false;
        let result = self.internal_unify_types(expected, other, span, mode, &mut mismatch);
        if mismatch {
            // with this mismatch flag, each unify can only trigger one error
            self.type_mismatch(expected, other, span);
        }
        result
    }

    #[track_caller]
    fn internal_unify_types(&mut self, expected: TypeId, other: TypeId, span: Span, mode: UnifyMode, mismatch: &mut bool) -> TypeId {
        let id_a = self.prune_id_once(expected);
        let id_b = self.prune_id_once(other);

        // already equal -> do nothing
        if id_a == id_b { return id_a }

        let a = self.type_arena.types[id_a.0 as usize].clone();
        let b = self.type_arena.types[id_b.0 as usize].clone();

        let mut new_mismatch = || {
            *mismatch = true;
            TypeId::ERROR
        };

        let result: TypeId = match (a, b) {
            (Type::Never, _) => id_b,
            (_, Type::Never) => id_a,

            // if one is an inference variable, bind it to the other type.
            (Type::Infer(id), _) => { self.inference_types[id.0 as usize] = Some(id_b); id_b }
            (_, Type::Infer(id)) => { self.inference_types[id.0 as usize] = Some(id_a); id_a }

            (Type::Error, _) | (_, Type::Error) => TypeId::ERROR,

            (Type::TupArr(inner_a, len_a), Type::TupArr(inner_b, len_b)) if len_a == len_b => {
                let merged = self.unify_types(inner_a, inner_b, span, mode);
                self.type_arena.add_type(Type::TupArr(merged, len_a))
            }

            (Type::Tup(elems_a), Type::Tup(elems_b)) if elems_a.len() == elems_b.len() => {
                let mut new_elems = Vec::new();

                for (ia, ib) in elems_a.into_iter().zip(elems_b) {
                    if ia.label != ib.label {
                        *mismatch = true;
                    }

                    let id = self.unify_types(ia.typ, ib.typ, span, mode);
                    new_elems.push(TypeTuple { label: ia.label, typ: id });
                }

                self.type_arena.add_type(Type::Tup(new_elems))
            }

            (Type::TupArr(inner_a, len_a), Type::Tup(elems_b)) if len_a == elems_b.len() => {
                let new_elems = elems_b.into_iter()
                    .map(|elem_a| TypeTuple {
                        label: elem_a.label,
                        typ: self.unify_types(elem_a.typ, inner_a, span, mode)
                    })
                    .collect();

                self.type_arena.add_type(Type::Tup(new_elems))
            }
            (Type::Tup(elems_a), Type::TupArr(inner_b, len_b)) if elems_a.len() == len_b => {
                let new_elems = elems_a.into_iter()
                    .map(|elem_a| TypeTuple {
                        label: elem_a.label,
                        typ: self.unify_types(elem_a.typ, inner_b, span, mode)
                    })
                    .collect();

                self.type_arena.add_type(Type::Tup(new_elems))
            }


            (Type::CustomType(id_a, inner_a), Type::CustomType(id_b, inner_b))
            if id_a == id_b => {
                let merged_inner = self.unify_types(inner_a, inner_b, span, mode);
                self.type_arena.add_type(Type::CustomType(id_a, merged_inner))

                // Different custom_id => they can't unify
                // e.g. `type N1 = num;  type N2 = num;  N1{ 2 } + N2{ 2 }`
            }

            (Type::Enum(enum_id_a), Type::Enum(enum_id_b)) if enum_id_a == enum_id_b => {
                id_a
            }

            (Type::EnumVariant { inner: inner_a, variant: variant_a },
            Type::EnumVariant { inner: inner_b, variant: variant_b }) => {
                let merged_inner = self.unify_types(inner_a, inner_b, span, mode);

                if variant_a == variant_b {
                    self.type_arena.add_type(Type::EnumVariant { inner: merged_inner, variant: variant_a })
                } else {
                    match mode {
                        UnifyMode::FindParentType => merged_inner,
                        UnifyMode::Subtype => new_mismatch()
                    }
                }
            }

            // this direction is always allowed regardless of mode
            (_, Type::EnumVariant { inner, .. }) => self.unify_types(id_a, inner, span, mode),
            (Type::EnumVariant { inner, .. }, _) => {
                // this direction is only allowed in LUB mode
                match mode {
                    UnifyMode::FindParentType => self.unify_types(inner, id_b, span, mode),
                    UnifyMode::Subtype => new_mismatch()
                }

            }


            (Type::Borrow { mutable: mut_a, inner: inner_a, borrows_var: borrows_a },
            Type::Borrow { mutable: mut_b, inner: inner_b, borrows_var: _ }) => {
                let new_inner = self.unify_types(inner_a, inner_b, span, mode);
                if mut_a != mut_b {
                    *mismatch = true;
                }
                self.type_arena.add_type(Type::Borrow { inner: new_inner, mutable: mut_a, borrows_var: borrows_a })
            }

            (Type::Fn { param_types: params_a, return_type: return_a },
            Type::Fn { param_types: params_b, return_type: return_b }) => {
                if params_a.len() != params_b.len() {
                    *mismatch = true;
                }
                let mut new_params = Vec::new();
                for (ia, ib) in params_a.into_iter().zip(params_b) {
                    new_params.push(self.unify_types(ib, ia, span, mode));
                }
                let new_return = self.unify_types(return_a, return_b, span, mode);

                self.type_arena.add_type(Type::Fn { param_types: new_params, return_type: new_return })
            }

            // any other case is a mismatch
            _ => new_mismatch()
        };

        result
    }

    fn unify_type_vec(&mut self, types: &[TypeId], span: Span) -> TypeId {
        if let Some((&first, others)) = types.split_first() {
            let mut merged = first;
            for &other in others {
                merged = self.unify_types(first, other, span, UnifyMode::FindParentType);
            }
            merged
        } else {
            self.new_infer_type()
        }
    }

    pub(super) fn are_types_equivalent(&self, left: TypeId, right: TypeId) -> bool {
        if left == right { return true }

        match (self.prune_type_once(left), self.prune_type_once(right)) {
            (Type::CustomType(left_id, left_inner), Type::CustomType(right_id, right_inner)) => {
                left_id == right_id && self.are_types_equivalent(left_inner, right_inner)
            }
            _ => false
        }
    }


    pub(super) fn fmt_type(&self, typ: TypeId) -> String {
        let mut s = String::new();
        self.write_type(typ, &mut s).unwrap();
        s
    }

    fn write_type(&self, id: TypeId, s: &mut String) -> fmt::Result {
        match self.type_arena.get_type(self.prune_id_once(id)) {
            Type::NumInt => write!(s, "int"),
            Type::NumFloat => write!(s, "float"),
            Type::Str => write!(s, "str"),
            Type::Bool => write!(s, "bool"),
            Type::Void => write!(s, "void"),
            Type::Never => write!(s, "never"),
            Type::MetaType => write!(s, "type"),
            Type::Error => write!(s, "error"),
            Type::Infer(infer) => write!(s, "?{}", infer.0),

            Type::Enum(enum_id) => {
                write!(s, "enum<{enum_id:?}>")
            }

            Type::CustomType(custom_id, _) => {
                write!(s, "{}", self.custom_types[custom_id.0 as usize].name)
            }

            Type::EnumVariant { inner, variant: variant_index } => {
                self.write_type(inner, s)?;

                let enum_id = self.get_wrapped_enum_id(inner).unwrap();
                let variants = &self.typed_ast.enum_defs[enum_id.0 as usize].variants;

                write!(s, ".{}", variants[variant_index].0)
            }

            Type::Tup(elems) => {
                write!(s, "tup<")?;
                for (i, elem) in elems.into_iter().enumerate() {
                    if i > 0 { write!(s, ", ")?; }

                    // Only print the label if it isn't an auto-generated number label
                    if !elem.label.as_bytes()[0].is_ascii_digit() {
                        write!(s, "{}: ", elem.label)?;
                    }
                    self.write_type(elem.typ, s)?;
                }
                write!(s, ">")
            }

            Type::TupArr(inner, length) => {
                write!(s, "tupArr<")?;
                self.write_type(inner, s)?;
                write!(s, "; {length}>")
            }

            Type::Fn { param_types, return_type } => {
                write!(s, "|")?;
                for (i, param) in param_types.into_iter().enumerate() {
                    if i > 0 { write!(s, ", ")?; }
                    self.write_type(param, s)?;
                }
                write!(s, " -> ")?;
                self.write_type(return_type, s)
            }

            Type::Borrow { inner, mutable, borrows_var } => {
                if mutable {
                    write!(s, "mut ")?;
                }
                write!(s, "ref ")?;
                self.write_type(inner, s)?;
                if let Some(var) = borrows_var {
                    write!(s, " ({})", self.typed_ast.get_var(var).name)?;
                }
                Ok(())
            }
        }
    }


    pub fn decay_soft_types(&mut self, id: TypeId) -> TypeId {
        let typ = self.prune_type_once(id);
        match typ {
            Type::EnumVariant { inner, variant: _ } => {
                self.decay_soft_types(inner)
            }
            _ => self.recursively_transform_type(id, Self::decay_soft_types)
        }
    }


    /// The final smoshing phase, also called zonking
    /// here its getting rid of all `Infer()` types. If it can't, it will throw a `TyperCantInferType` error
    /// (num. Infer(0)) -> (num, num)
    pub(super) fn finalize_types_and_vars(&mut self) {
        // cache to prevent recursively zonking the same type thousands of times.
        let mut cache = vec![None; self.type_arena.types.len()];

        // zonk Expression types
        for i in 0..self.typed_ast.expr_types.len() {
            let span = self.ast.expr_spans[i];
            self.typed_ast.expr_types[i] = self.zonk_type(self.typed_ast.expr_types[i], span, &mut cache);
        }
        // zonk Variable types
        for i in 0..self.typed_ast.vars.len() {
            let span = self.typed_ast.vars[i].declared_at;
            self.typed_ast.vars[i].typ = self.zonk_type(self.typed_ast.vars[i].typ, span, &mut cache);
        }
        // zonk enum variant types
        for i in 0..self.typed_ast.enum_defs.len() {
            for j in 0..self.typed_ast.enum_defs[i].variants.len() {
                self.typed_ast.enum_defs[i].variants[j].1 = self.zonk_type(self.typed_ast.enum_defs[i].variants[j].1, Span::invalid(), &mut cache);
            }
        }

        // warnings for unused vars
        if self.error_data.errors.is_empty() {
            for var in &self.typed_ast.vars {
                // don't warn for Prelude stuff, like `Range.iter()`
                if var.declared_at.byte_offset < crate::PRELUDE.len() || var.declared_at.byte_offset == usize::MAX {
                    continue
                }

                match var.is_used {
                    TypeVarIsUsed::No if !var.name.starts_with('_') => {
                        self.error_data.warn(WarnType::TyperUnusedVar { var: var.clone() }, var.declared_at);
                    }
                    TypeVarIsUsed::Immut if var.is_declared_mut => {
                        self.error_data.warn(WarnType::TyperUnusedMutVar { var: var.clone() }, var.declared_at);
                    }
                    _ => {}
                }
            }
        }
    }

    fn zonk_type(&mut self, id: TypeId, span: Span, cache: &mut Vec<Option<TypeId>>) -> TypeId {
        if let Some(zonked) = cache[id.0 as usize] {
            return zonked;
        }

        let typ = self.type_arena.types[id.0 as usize].clone();

        let zonked_id = match typ {
            // try to resolve this Type::Infer!
            Type::Infer(infer_id) => {
                if let Some(resolved_id) = self.inference_types[infer_id.0 as usize] {
                    // resolved! keep recursively zonking.
                    self.zonk_type(resolved_id, span, cache)
                } else {
                    self.error(ErrType::TyperCantInferType { typ: self.fmt_type(id) }, span)
                }
            }

            _ => self.recursively_transform_type(id, |tc, inner| {
                tc.zonk_type(inner, span, cache)
            })
        };

        // `self.type_arena.add_type()` might have added stuff
        if cache.len() < self.type_arena.types.len() {
            cache.resize(self.type_arena.types.len(), None);
        }
        cache[id.0 as usize] = Some(zonked_id);
        zonked_id
    }


    fn recursively_transform_type(&mut self, typ: TypeId, mut trans: impl FnMut(&mut Self, TypeId) -> TypeId) -> TypeId {
        match self.prune_type_once(typ) {
            Type::CustomType(custom_id, inner) => {
                let inner = trans(self, inner);
                self.type_arena.add_type(Type::CustomType(custom_id, inner))
            }
            Type::Tup(elems) => {
                let elems = elems.into_iter().map(|e| TypeTuple { label: e.label, typ: trans(self, e.typ) }).collect();
                self.type_arena.add_type(Type::Tup(elems))
            }
            Type::TupArr(typ, len) => {
                let typ = trans(self, typ);
                self.type_arena.add_type(Type::TupArr(typ, len))
            }
            Type::Borrow { inner, mutable, borrows_var } => {
                let inner = trans(self, inner);
                self.type_arena.add_type(Type::Borrow { inner, mutable, borrows_var })
            }
            Type::EnumVariant { inner, variant } => {
                let inner = trans(self, inner);
                self.type_arena.add_type(Type::EnumVariant { inner, variant })
            }
            Type::Fn { param_types, return_type } => {
                let param_types = param_types.into_iter().map(|p| trans(self, p)).collect();
                let return_type = trans(self, return_type);
                self.type_arena.add_type(Type::Fn { param_types, return_type })
            }

            Type::NumInt | Type::NumFloat | Type::Str | Type::Bool | Type::Void | Type::Never
            | Type::MetaType | Type::Error | Type::Enum(_)
            | Type::Infer(_) => typ,
        }
    }
}