use std::collections::HashMap;

use derive_more::Display;

use crate::{
    ErrType, lexing::tokens::Span, nativelib::ThrumModule, parsing::ast::{AstEnumExpression, Expr, ExprId, PatternId},
    typing::{CustomTypeId, EnumDefinition, EnumId, LabelInfo, Type, TypeChecker, TypeId, TypeVarId, check_expressions::CheckExprCtx},
    vm_compiling::{VmValue, VmCompiler}
};



/// The current scope
/// `let x = 5` would insert "x"
#[derive(Debug, Default)]
pub struct TypeVarScope<'a> {
    pub scope: HashMap<&'a str, TypeVarId>,
}


#[derive(Debug, Display, Clone)]
#[display("{name}: {typ:?}")]
pub struct TypeVar {
    pub name: String,
    pub typ: TypeId,

    pub declared_at: Span,  // Source code location - for error messages
    pub is_declared_mut: bool,
    pub is_init: TypeVarMemState,
    pub const_val: TypeVarConstVal,

    // unused variables warnings
    pub is_used: TypeVarIsUsed,

    // borrow counting
    pub immut_borrows_count: usize,
    pub mut_borrows_count: usize,
}

#[derive(Debug, Clone)]
pub enum TypeVarConstVal {
    /// its a runtime variable
    No,
    // all others are consts
    NotYetTypechecked { value: ExprId, bind_to: PatternOrVarId },
    CurrTypechecking,
    NotYetEvaluated { value: ExprId, bind_to: PatternOrVarId },
    Evaluated(VmValue),
}
#[derive(Debug, Clone, Copy)]
pub enum PatternOrVarId {
    Pattern(PatternId, DefineVarScope),
    CustomTypeVarId(TypeVarId),
}

/// This enum tracks if a `TypeVar` is initialized or not.
///
/// Maybe happens when one if-branch initializes a var and the other branch doesnt.\
/// (`NotInit` / `Moved`) and (`MaybeInit` / `MaybeMoved`) are effectively the same, just for different error messages
#[derive(Clone, Copy, Debug)]
pub enum TypeVarMemState {
    NotInit, Init, MaybeInit,
    Moved, MaybeMoved,
}

#[derive(Clone, Debug)]
pub enum TypeVarIsUsed {
    No, Immut, Mut
}

pub enum DefineVarMode {
    Const(TypeVarConstVal),
    Var { mutable: bool, is_init: bool },
}
#[derive(Debug, Clone, Copy)]
pub enum DefineVarScope { CurrScope, Impl { typ: TypeId } }


pub type SnapshotVarsState = HashMap<TypeVarId, TypeVarMemState>;


impl<'ast> TypeChecker<'ast> {
    // when entering a block a new scope gets added
    pub(super) fn enter_scope(&mut self) {
        self.var_scopes.push(TypeVarScope::default());
    }
    pub(super) fn exit_scope(&mut self) {
        self.var_scopes.pop().unwrap();
    }

    pub(super) fn define_variable(&mut self, name: &'ast str, typ: TypeId, explicit_type_anot: bool, span: Span, define_scope: DefineVarScope, define_mode: DefineVarMode) -> TypeVarId {
        // a var cant be shadowed if its a const
        let (mutable, is_init, const_val, shadowable) = match define_mode {
            DefineVarMode::Var { mutable, is_init } => (mutable, is_init, TypeVarConstVal::No, true),
            DefineVarMode::Const(const_val) => (false, true, const_val, false)
        };

        let final_type = if explicit_type_anot { typ } else { self.decay_soft_types(typ) };

        let new_var = TypeVar {
            typ: final_type,
            const_val,
            name: name.to_string(),
            declared_at: span,
            is_declared_mut: mutable,
            is_init: if is_init { TypeVarMemState::Init } else { TypeVarMemState::NotInit },
            is_used: TypeVarIsUsed::No,
            immut_borrows_count: 0,
            mut_borrows_count: 0,
        };

        let var_id = TypeVarId(self.typed_ast.vars.len().try_into().unwrap());
        self.typed_ast.vars.push(new_var);

        let scope = match define_scope {
            DefineVarScope::CurrScope => self.var_scopes.last_mut().unwrap(),
            DefineVarScope::Impl { typ } => self.type_impls.entry(typ).or_default()
        };
        let previous_val = scope.scope.insert(name, var_id);

        if !shadowable && previous_val.is_some() {
            self.error(ErrType::TyperConstNameAlreadyExists { name: name.to_string() }, span);
        }

        var_id
    }


    pub(super) fn lookup_variable(&mut self, name: &str) -> Option<TypeVarId> {
        for i in (0..self.var_scopes.len()).rev() {
            if let Some(&var_id) = self.var_scopes[i].scope.get(name) {
                // on lookup ensure that the var was typechecked and compiled
                self.resolve_const_val(var_id);
                return Some(var_id)
            }
        }
        None
    }

    fn add_enum_def(&mut self, def: EnumDefinition) -> EnumId {
        let id = EnumId(self.typed_ast.enum_defs.len().try_into().unwrap());
        self.typed_ast.enum_defs.push(def);
        id
    }


    pub(super) fn resolve_const_val(&mut self, var_id: TypeVarId) {
        let var = self.typed_ast.get_var(var_id);
        match var.const_val {
            TypeVarConstVal::NotYetTypechecked { value, bind_to } => {
                // and resolve it!
                self.check_evaluate_and_bind_const(value, bind_to);
            }
            TypeVarConstVal::CurrTypechecking => {
                self.error(ErrType::TyperConstResolvingCycle, var.declared_at);
            }
            TypeVarConstVal::NotYetEvaluated { value, bind_to } => {
                self.evaluate_and_bind_const(value, bind_to);
            }
            TypeVarConstVal::No
            | TypeVarConstVal::Evaluated(_) =>  {/* all good, do nothing */}
        }
    }

    pub(super) fn check_evaluate_and_bind_const(&mut self, value: ExprId, bind_to: PatternOrVarId) {
        let impl_self = match bind_to {
            PatternOrVarId::Pattern(_, DefineVarScope::Impl { typ }) => Some(typ),
            _ => None,
        };
        if let Some(typ) = impl_self {
            self.curr_impl_self.push(typ);
        }

        // mark the pattern as curr typechecking, so it can detect cycles
        match bind_to {
            PatternOrVarId::Pattern(pattern, define_scope) => {
                self.mark_vars_in_pattern_as_const(pattern, TypeVarConstVal::CurrTypechecking, define_scope);
            }
            PatternOrVarId::CustomTypeVarId(var_id) => {
                self.typed_ast.get_var_mut(var_id).const_val = TypeVarConstVal::CurrTypechecking;
            }
        }

        // typecheck it:
        // remove these while typechecking consts so it can't break/return
        let prev_fn = self.curr_function_return_type.take();
        let prev_labels = std::mem::take(&mut self.curr_label_infos);

        match bind_to {
            PatternOrVarId::Pattern(pattern, _) => {
                self.check_assign_pattern_and_value(
                    pattern, Some(value), &mut false, true, false, false,
                    Some(TypeVarConstVal::NotYetEvaluated { value, bind_to })
                );
            }
            PatternOrVarId::CustomTypeVarId(var_id) => {
                self.check_expression(value, &mut false, CheckExprCtx::default().expect(TypeId::TYPE));
                self.typed_ast.get_var_mut(var_id).const_val = TypeVarConstVal::NotYetEvaluated { value, bind_to };
            }
        }

        self.curr_function_return_type = prev_fn;
        self.curr_label_infos = prev_labels;


        // evaluate and bind it:
        self.evaluate_and_bind_const(value, bind_to);

        if impl_self.is_some() {
            assert_eq!(self.curr_impl_self.pop(), impl_self);
        }
    }



    pub(super) fn evaluate_and_bind_const(&mut self, value: ExprId, bind_to: PatternOrVarId) {
        let value_expr = self.ast.get_expr(value);
        let evaluated = match value_expr {
            Expr::Closure { .. } => {
                // closures can have cyclic dependencies (recursion), so it
                // needs to first bind to the pattern, and AFTER typecheck
                Some(VmValue::Fn {
                    slot: self.typed_ast.resolved_closure_fn_id[&value]
                })
            }

            _ => self.evaluate_expr(value)
        };

        if let Some(val) = evaluated {
            match bind_to {
                PatternOrVarId::Pattern(pattern, define_scope) => {
                    self.mark_vars_in_pattern_as_const(pattern, TypeVarConstVal::Evaluated(val), define_scope);
                }

                // e.g. `type X = int`
                PatternOrVarId::CustomTypeVarId(var_id) => {
                    let VmValue::Type(meta_type_id) = val else {
                        unreachable!("not a meta type?! {val}")
                    };

                    // add the new CustomType!!
                    let new_type_id = CustomTypeId(self.custom_types.len().try_into().unwrap());
                    let var_name = self.typed_ast.get_var(var_id).name.clone().into_boxed_str();
                    self.custom_types.push(var_name);

                    let new_type = self.type_arena.add_type(Type::CustomType(new_type_id, meta_type_id));
                    let type_const = VmValue::Type(new_type);

                    self.typed_ast.get_var_mut(var_id).const_val = TypeVarConstVal::Evaluated(type_const);
                }
            }
        }

        if let Expr::Closure { closure, .. } = value_expr {
            // AFTER binding typecheck the body
            let fn_type = self.typed_ast.get_expr_type(value);
            self.check_fn_expression(closure, fn_type);
        }
    }



    /// this function actually evaluates a const-expr
    /// in the typechecker itself this is used in:
    /// `const x = ...`
    /// `let x: ... = 5`
    /// `(0; ...)`
    pub(super) fn evaluate_expr(&mut self, expr: ExprId) -> Option<VmValue> {
        // if there were any errors, don't evaluate consts anymore.
        // this fixes compiler crashes, but its definitely not the best
        if !self.error_data.errors.is_empty() {
            // TODO: make better
            return None
        }

        match self.ast.get_expr(expr) {
            // also needs to handle EnumDefinitions
            Expr::EnumDefinition { variants } => {
                let variants = variants.iter()
                    .map(|AstEnumExpression { variant_name, attached_tuple }| {
                        let attached_tuple_type = attached_tuple.map_or_else(
                            || TypeId::VOID,
                            |tup| self.check_annotation_meta_type_id(tup, false)
                        );
                        (variant_name.clone(), attached_tuple_type)
                    }).collect();

                let enum_id = self.add_enum_def(EnumDefinition { variants });
                let enum_type = self.type_arena.add_type(Type::Enum(enum_id));

                Some(VmValue::Type(enum_type))
            }

            _ => {
                match VmCompiler::compile_and_run_comptime_expr(self.ast, &self.typed_ast, &mut self.type_arena, &mut self.compiled_functions, expr) {
                    Ok(val) => Some(val),
                    Err(e) => {
                        self.error(e, self.ast.get_expr_span(expr));
                        None
                    }
                }
            }
        }
    }




    pub(super) fn make_variable_ref(&mut self, name: &str, mutable: bool, is_const: bool, expr: ExprId) -> TypeId {
        let expr_span = self.ast.get_expr_span(expr);

        if let Some(var_id) = self.lookup_variable(name) {
            self.typed_ast.resolved_expr_var.insert(expr, var_id);

            if is_const && let TypeVarConstVal::No = self.typed_ast.get_var(var_id).const_val {
                self.error(ErrType::TyperExpectedConstFoundRuntimeValue { name: name.to_string() }, expr_span)
            } else {
                self.make_var_id_ref(var_id, mutable)
            }
        } else {
            self.error(ErrType::TyperUndefinedIdentifier { name: name.to_string() }, expr_span)
        }
    }

    pub(super) fn make_var_id_ref(&mut self, var_id: TypeVarId, mutable: bool) -> TypeId {
        let var = self.typed_ast.get_var_mut(var_id);
        let inner = var.typ;

        if mutable {
            // if var.borrows_count > 0 { errors.push(ErrType::TyperCantBorrowMutBecauseAlreadyBorrowed); }
            // if var.mut_borrows_count > 0 { errors.push(ErrType::TyperCantBorrowMutBecauseAlreadyBorrowedMut); }
            var.mut_borrows_count += 1;

            // if !var.is_declared_mut {
            //     let var = var.clone();
            //     let span = var.declared_at;
            //     self.error(ErrType::TyperVarIsntDeclaredMut { var }, span);
            // }
        } else {
            // if var.mut_borrows_count > 0 { errors.push(ErrType::TyperCantBorrowBecauseAlreadyBorrowedMut); }
            var.immut_borrows_count += 1;
        }

        if mutable {
            var.is_used = TypeVarIsUsed::Mut;
        } else if let TypeVarIsUsed::No = var.is_used {
            var.is_used = TypeVarIsUsed::Immut;
        }

        self.type_arena.add_type(Type::Borrow { mutable, inner, borrows_var: Some(var_id) })
    }


    pub(super) fn update_variable(&mut self, var_id: TypeVarId, span: Span) {
        let var = self.typed_ast.get_var_mut(var_id);
        let mut errors = Vec::new();

        var.is_used = TypeVarIsUsed::Mut;

        if !var.is_declared_mut {
            match var.is_init {
                TypeVarMemState::NotInit => { /* perfectly fine, do nothing */ },
                TypeVarMemState::MaybeInit => errors.push(ErrType::TyperCantUseMaybeInitializedVar { var: var.clone() }),
                TypeVarMemState::Init | TypeVarMemState::MaybeMoved | TypeVarMemState::Moved => errors.push(ErrType::TyperVarIsntDeclaredMut { var: var.clone() })
            }
        }
        var.is_init = TypeVarMemState::Init;  // if variable was moved, doing `a = ...` unmoves it again.

        for e in errors { self.error(e, span); }
    }

    pub(super) fn clone_variable(&mut self, var_id: TypeVarId, span: Span) {
        let var = self.typed_ast.get_var_mut(var_id);
        let mut errors = Vec::new();

        match var.is_init {
            TypeVarMemState::Moved      => errors.push(ErrType::TyperCantUseMovedVar            { var: var.clone() }),
            TypeVarMemState::NotInit    => errors.push(ErrType::TyperCantUseUninitializedVar    { var: var.clone() }),
            TypeVarMemState::MaybeMoved => errors.push(ErrType::TyperCantUseMaybeMovedVar       { var: var.clone() }),
            TypeVarMemState::MaybeInit  => errors.push(ErrType::TyperCantUseMaybeInitializedVar { var: var.clone() }),
            TypeVarMemState::Init       => { /* perfectly fine, do nothing */ },
        }

        for e in errors { self.error(e, span); }
    }

    pub(super) fn move_variable(&mut self, var_id: TypeVarId, span: Span) {
        self.clone_variable(var_id, span);
        let var = self.typed_ast.get_var_mut(var_id);
        var.is_init = TypeVarMemState::Moved;
    }





    // logic for "definite assignment analysis", for example:
    // let x
    // if false { x = 5 }
    // println!("{x}")  //-> Error, because x is not initialized in all branches of the if statement
    pub(super) fn snapshot_vars_state(&self) -> SnapshotVarsState {
        // iterate over all currently in scope variables
        let mut vars_state = HashMap::new();
        for scope in &self.var_scopes {
            for &var_id in scope.scope.values() {
                let var_init = self.typed_ast.get_var(var_id).is_init;
                vars_state.insert(var_id, var_init);
            }
        }
        vars_state
    }
    pub(super) fn snapshot_branch_vars_state(&self, branch_was_never: bool) -> Option<SnapshotVarsState> {
        // if the branch has type Never it should not be included in the later merge
        (!branch_was_never).then(|| self.snapshot_vars_state())
    }

    pub(super) fn restore_vars_state(&mut self, snap: &SnapshotVarsState) {
        for (&var_id, is_init) in snap {
            self.typed_ast.get_var_mut(var_id).is_init = *is_init;
        }
    }

    pub(super) fn merge_vars_states(&mut self, original_snap: SnapshotVarsState, branch_snaps: &[Option<SnapshotVarsState>]) {
        for original_snap_var_id in original_snap.into_keys() {
            let mut any_branch_init = false;
            let mut all_branches_init = true;
            let mut was_moved = false;

            // filter the None's out (the branches that had Never type)
            for branch_snap in branch_snaps.iter().filter_map(|x| x.as_ref()) {
                let branch_state = branch_snap.get(&original_snap_var_id).unwrap();

                if let TypeVarMemState::MaybeInit | TypeVarMemState::MaybeMoved | TypeVarMemState::Init = branch_state { any_branch_init = true; }
                if let TypeVarMemState::MaybeInit | TypeVarMemState::MaybeMoved | TypeVarMemState::Moved | TypeVarMemState::NotInit = branch_state { all_branches_init = false; }
                if let TypeVarMemState::MaybeMoved | TypeVarMemState::Moved = branch_state { was_moved = true; }
            }

            self.typed_ast.get_var_mut(original_snap_var_id).is_init = match (all_branches_init, any_branch_init, was_moved) {
                // every single branch initialized this var -> var is initialized.
                (true, true, _) => TypeVarMemState::Init,

                // some branches did, some didn't -> uncertain...
                (false, true, moved) | (true, false, moved) => if moved { TypeVarMemState::MaybeMoved } else { TypeVarMemState::MaybeInit },

                // no branch touched it -> keep NotInit/Moved
                (false, false, false) => TypeVarMemState::NotInit,
                (false, false, true) => TypeVarMemState::Moved,
            }
        }
    }



    pub(super) fn before_check_label_logic(&mut self, expr: ExprId, label: &'ast str, typ: TypeId) -> SnapshotVarsState {
        self.curr_label_infos.push(LabelInfo { label, expr, typ, break_snapshots: Vec::new() });
        self.snapshot_vars_state()
    }



    pub(super) fn find_loop_label(&mut self, label: Option<&str>, span: Span) -> Option<&mut LabelInfo<'ast>> {
        if self.curr_label_infos.is_empty() {
            self.error(ErrType::TyperBreakOutsideLoop, span);
            None
        }
        else if let Some(target_label) = label {
            // rposition to search from the back (innermost loop out).
            let found_index = self.curr_label_infos.iter()
                .rposition(|info| info.label == target_label);

            if let Some(idx) = found_index {
                // found the label, return the break type
                Some(&mut self.curr_label_infos[idx])
            } else {
                // couldn't find the label -> report error
                self.error(ErrType::TyperUndefinedLoopLabel {
                    label: target_label.to_string(),
                    available: self.curr_label_infos.iter().map(|info| info.label.to_string()).collect()
                }, span);

                None
            }
        } else {
            // Break without label -> grab the last one
            Some(self.curr_label_infos.last_mut().unwrap())
        }
    }


    pub(super) fn load_prelude_from_lib(&mut self, module: &'ast ThrumModule) {
        for (name, value) in &module.values {
            if value.is_prelude {
                let id = self.type_arena.add_type(value.typ.clone());
                self.define_variable(
                    name, id, true, Span::invalid(), DefineVarScope::CurrScope,
                    DefineVarMode::Const(TypeVarConstVal::Evaluated(value.val.clone()))
                );
            }
        }
        // Recursion
        for sub_module in module.sub_modules.values() {
            self.load_prelude_from_lib(sub_module);
        }
    }
}