use crate::{
    ErrType, lexing::tokens::Span,
    parsing::ast::ExprId,
    typing::{Type, TypeChecker, TypeId, TypeVarId }
};


#[derive(Default, Clone, Copy)]
pub enum AutoDerefMode {
    #[default]
    None, Once, Fully, LeaveOnePointer
}

impl TypeChecker<'_> {
    pub(super) fn coerce_to_expected_type(&mut self, expr: ExprId, expected_id: TypeId) -> TypeId {
        // 1. deref pointers so the amounts there match.
        self.auto_deref_to_expected_type(expr, expected_id);

        // 2. coercion.
        let coerced = self.coerce_type(expr, expected_id);
        self.typed_ast.expr_types[expr.0 as usize] = coerced;
        coerced
    }



    fn coerce_type(&mut self, expr: ExprId, expected_id: TypeId) -> TypeId {
        let expr_type_id = self.typed_ast.get_expr_type(expr);
        let expr_type = self.prune_type_once(expr_type_id);
        let expected_type = self.prune_type_once(expected_id);

        match (expr_type, expected_type) {
            // Tuple -> MetaType
            (Type::Tup(elems), Type::MetaType) => {
                let labels = elems.into_iter().map(|elem| elem.label).collect();
                self.typed_ast.resolved_tuple_type_coerce.insert(expr, labels);
                TypeId::TYPE
            }
            // TupArr -> MetaType
            (Type::TupArr(_, len), Type::MetaType) => {
                let labels = (0..len).map(|x| x.to_string()).collect();
                self.typed_ast.resolved_tuple_type_coerce.insert(expr, labels);
                TypeId::TYPE
            }

            _ => expr_type_id
        }
    }




    /// this function allows auto-deref to autoclone types\
    /// e.g. expr.typ: &&bool, expected: bool -> works, inserts 2 copies (only because bool is `auto_clone`)\
    /// e.g. expr.typ: &&Vec, expected: &Vec -> works, inserts 1 copy
    fn auto_deref_to_expected_type(&mut self, expr: ExprId, expected_id: TypeId) -> TypeId {
        let expr_type = self.typed_ast.get_expr_type(expr);

        if let Some((expr_p_count, is_auto_clone_after, _)) = self.count_initial_pointers(expr_type)
        && let Some((expected_p_count, _, _)) = self.count_initial_pointers(expected_id) {

            // if we have more pointers than the expected_type, AND everything that needs to be derefed is auto_clone
            if expr_p_count > expected_p_count && (expected_p_count != 0 || is_auto_clone_after) {
                for _ in 0..(expr_p_count - expected_p_count) {
                    assert!(self.deref_if_pointer(expr));
                }
            }
        }

        self.typed_ast.get_expr_type(expr)
    }

    pub(super) fn count_initial_pointers(&self, typ: TypeId) -> Option<(usize, bool, TypeId)> {
        let mut curr_typ = typ;
        let mut count = 0;

        while let Type::Borrow { inner, .. } = self.prune_type_once(curr_typ) {
            curr_typ = inner;
            count += 1;
        }

        self.is_auto_clone(curr_typ)
            .map(|final_auto_clone| (count, final_auto_clone, curr_typ))
    }



    pub(super) fn handle_deref_mode(&mut self, deref_mode: AutoDerefMode, check_expr: ExprId) -> TypeId {
        match deref_mode {
            AutoDerefMode::None => {/* do nothing */}
            AutoDerefMode::Once => {
                self.deref_if_pointer(check_expr);
            }
            AutoDerefMode::Fully => {
                while self.deref_if_pointer(check_expr) { }
            }
            AutoDerefMode::LeaveOnePointer => {
                if let Some((p_count, _, _)) = self.count_initial_pointers(self.typed_ast.get_expr_type(check_expr)) {
                    for _ in 1..p_count {
                        assert!(self.deref_if_pointer(check_expr));
                    }
                }
            }
        }
        self.typed_ast.get_expr_type(check_expr)
    }


    pub(super) fn deref_if_pointer(&mut self, expr: ExprId) -> bool {
        let span = self.ast.get_expr_span(expr);
        let typ = self.typed_ast.get_expr_type(expr);

        match self.prune_type_once_infer_err(typ, span) {
            Type::Borrow { inner, mutable: _, borrows_var } => {

                *self.typed_ast.auto_derefs.entry(expr).or_default() += 1;
                self.typed_ast.expr_types[expr.0 as usize] = inner;

                let auto_clone = self.check_deref_memory_rules(inner, borrows_var, span);
                if !auto_clone {
                    self.typed_ast.move_expr.insert(expr);
                }
                // println!("derefed: {:?}", self.ast.get_expr(expr));
                true
            }
            _ => false
        }
    }

    pub(super) fn check_deref_memory_rules(&mut self, inner: TypeId, borrows_var: Option<TypeVarId>, span: Span) -> bool {
        if let Some(auto_clone) = self.is_auto_clone(inner) {
            if auto_clone {
                if let Some(x) = borrows_var {
                    self.clone_variable(x, span);
                }
            } else {
                // if it isn't autoclone, it needs to move it
                if let Some(x) = borrows_var {
                    self.move_variable(x, span);
                }
                else {
                    self.error(ErrType::TyperCantDerefUnknownPointerType, span);
                }
            }
            auto_clone
        } else {
            self.error(ErrType::TyperTypeMustBeKnownHere { typ: self.fmt_type(inner) }, span);
            true  // just return true here for less chained errors
        }
    }

    /// Some(true) - is autoclone
    /// Some(false) - isn't autoclone
    /// None - infer error
    fn is_auto_clone(&self, typ: TypeId) -> Option<bool> {
        match self.prune_type_once(typ) {
            Type::NumInt
            | Type::NumFloat
            | Type::Bool
            | Type::Borrow { .. }
            | Type::Fn { .. }
            | Type::MetaType
            | Type::Void
            | Type::Never
            | Type::Error => Some(true),

            Type::Str
            | Type::Enum(_) => Some(false),

            Type::Tup(inners) => {
                let mut all_clone = true;
                for tt in inners {
                    match self.is_auto_clone(tt.typ) {
                        Some(true) => {},
                        Some(false) => all_clone = false,
                        None => return None,
                    }
                }
                Some(all_clone)
            }

            Type::TupArr(inner, _)
            | Type::CustomType(_, inner)
            | Type::EnumVariant { inner, .. } => {
                self.is_auto_clone(inner)
            }

            Type::Infer(_) => None
        }
    }
}