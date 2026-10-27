use crate::{
    ErrType, lexing::tokens::Span, parsing::ast::ExprId, typing::{Lifetime, TupMode, Type, TypeChecker, TypeId, UnifyMode }
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
        self.coerce_type(expr, expected_id)
    }



    fn coerce_type(&mut self, expr: ExprId, expected_id: TypeId) -> TypeId {
        let expr_type_id = self.typed_ast.get_expr_type(expr);
        let expr_type = self.prune_type_once(expr_type_id);
        let expected_type = self.prune_type_once(expected_id);

        let coerced = match (expr_type, expected_type) {
            // Tuple -> MetaType
            (Type::Tup(mode), Type::MetaType) => {
                let labels = match mode {
                    TupMode::Tup(elems) => elems.into_iter().map(|elem| elem.label).collect(),
                    TupMode::Arr(_, len) => (0..len).map(|x| x.to_string()).collect()
                };
                self.typed_ast.resolved_tuple_type_coerce.insert(expr, labels);
                TypeId::TYPE
            }

            // &tuple -> &[T]
            (Type::Borrow { inner: inner_expr, mutable: mut_expr, lifetime: life_expr },
            Type::Borrow { inner: inner_expected, mutable: mut_expected, lifetime: life_expected })
            if let Type::Tup(tup_mode) = self.prune_type_once(inner_expr)
            && let Some(elem_expr) = self.get_tuple_inner_type_if_homogenous(&tup_mode)
            && let Type::Slice(elem_expected) = self.prune_type_once(inner_expected)
            && self.are_types_equivalent(elem_expr, elem_expected)
            && (!mut_expected || mut_expr) => {

                self.typed_ast.resolved_tuple_slice_coerce.insert(expr, tup_mode.len());
                let lifetime = self.unify_lifetimes(life_expr, life_expected);

                self.type_arena.add_type(Type::Borrow { inner: inner_expected, mutable: mut_expected, lifetime })
            }

            // &str -> FmtStr
            (Type::Borrow { inner, .. }, Type::FmtStr)
            if let Type::Str = self.prune_type_once(inner) => {
                self.typed_ast.resolved_format_str_coerce.insert(expr);
                TypeId::FMT_STR
            }

            _ => expr_type_id
        };

        self.typed_ast.expr_types[expr.0 as usize] = coerced;
        coerced
    }




    /// this function allows auto-deref to autoclone types\
    /// e.g. expr.typ: &&bool, expected: bool -> works, inserts 2 copies (only because bool is `auto_clone`)\
    /// e.g. expr.typ: &&Vec, expected: &Vec -> works, inserts 1 copy
    fn auto_deref_to_expected_type(&mut self, expr: ExprId, expected_id: TypeId) -> TypeId {
        let expr_type = self.typed_ast.get_expr_type(expr);

        let (expr_p_count, expr_base) = self.count_borrow_layers(expr_type);
        let (expected_p_count, _) = self.count_borrow_layers(expected_id);
        let is_auto_clone_after = self.is_auto_clone(expr_base).unwrap_or(false);

        // if we have more pointers than the expected_type, AND everything that needs to be derefed is auto_clone
        if expr_p_count > expected_p_count && (expected_p_count != 0 || is_auto_clone_after) {
            for _ in 0..(expr_p_count - expected_p_count) {
                assert!(self.deref_if_pointer(expr));
            }
        }

        self.typed_ast.get_expr_type(expr)
    }

    pub(super) fn count_borrow_layers(&self, mut typ: TypeId) -> (usize, TypeId) {
        let mut count = 0;
        while let Type::Borrow { inner, .. } = self.prune_type_once(typ) {
            typ = inner;
            count += 1;
        }
        (count, typ)
    }

    pub(super) fn count_derefable_pointers(&self, typ: TypeId) -> Option<(usize, bool, TypeId)> {
        let mut curr_typ = typ;
        let mut count = 0;

        while let Some((inner, _)) = self.is_derefable_pointer(curr_typ) {
            curr_typ = inner;
            count += 1;
        }

        // returns None on inference error
        self.is_auto_clone(curr_typ)
            .map(|final_auto_clone| (count, final_auto_clone, curr_typ))
    }

    pub(super) fn is_derefable_pointer(&self, typ: TypeId) -> Option<(TypeId, Lifetime)> {
        if let Type::Borrow { inner, mutable: _, lifetime } = self.prune_type_once(typ)
        && let Some(false) = self.is_unsized(inner) {
            Some((inner, lifetime))
        } else {
            None
        }
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
                let (p_count, _) = self.count_borrow_layers(self.typed_ast.get_expr_type(check_expr));
                for _ in 1..p_count {
                    assert!(self.deref_if_pointer(check_expr));
                }
            }
        }
        self.typed_ast.get_expr_type(check_expr)
    }


    pub(super) fn deref_if_pointer(&mut self, expr: ExprId) -> bool {
        let span = self.ast.get_expr_span(expr);
        let typ = self.typed_ast.get_expr_type(expr);
        let typ = self.prune_id_once_infer_err(typ, span);

        if let Some((inner, lifetime)) = self.is_derefable_pointer(typ) {

            *self.typed_ast.auto_derefs.entry(expr).or_default() += 1;
            self.typed_ast.expr_types[expr.0 as usize] = inner;

            let auto_clone = self.check_deref_memory_rules(inner, lifetime, span);
            if !auto_clone {
                self.typed_ast.move_expr.insert(expr);
            }
            // println!("derefed: {:?}", self.ast.get_expr(expr));
            true
        } else {
            false
        }
    }

    pub(super) fn check_deref_memory_rules(&mut self, inner: TypeId, lifetime: Lifetime, span: Span) -> bool {
        if let Some(auto_clone) = self.is_auto_clone(inner) {
            if auto_clone {
                if let Lifetime::LocalVariable(x) = lifetime {
                    self.clone_variable(x, span);
                }
            } else {
                // if it isn't autoclone, it needs to move it
                if let Lifetime::LocalVariable(x) = lifetime {
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


    pub(super) fn get_tuple_inner_type_if_homogenous(&mut self, tup_mode: &TupMode) -> Option<TypeId> {
        match tup_mode {
            &TupMode::Arr(inner, _) => Some(inner),
            TupMode::Tup(elems) => if let Some(first) = elems.first() {
                let first_typ = first.typ;
                if elems.iter().all(|e| self.are_types_equivalent(e.typ, first_typ)) {
                    Some(first_typ)
                } else {
                    None
                }
            } else {
                Some(self.new_infer_type())
            }
        }
    }

    pub(super) fn unify_branch_types(&mut self, branches: &[(ExprId, TypeId, bool/* is_never */)], span: Span) -> TypeId {
        // filter out never-branches
        let active: Vec<(ExprId, TypeId)> = branches.iter()
            .filter_map(|&(expr, typ, is_never)| {
                (!is_never && self.prune_id_once(typ) != TypeId::NEVER).then_some((expr, typ))
            })
            .collect();

        match active.as_slice() {
            // easy cases:
            [] => TypeId::NEVER,
            [(_, single_type)] => *single_type,

            // hard case...
            [(_, target), rest @ ..] => {
                // THE target type for all branches
                let mut target = *target;

                // if any branch is a slice reference, it wins!
                // (because &(T; N) can coerce to &[T], but &[T] can't coerce to &(T; N))
                let mut slice_target = self.get_slice_ref_type(target);
                for &(_, next_type) in rest {
                    if let Some(slice_ref) = self.get_slice_ref_type(next_type) {
                        slice_target = Some(slice_ref);
                        break;
                    }
                }

                if let Some(slice_ref) = slice_target {
                    target = slice_ref;
                } else {
                    // otherwise, find the common target by reference depth & LUB
                    for &(_, next_type) in rest {
                        let (target_borrows, _) = self.count_borrow_layers(target);
                        let (next_borrows, _) = self.count_borrow_layers(next_type);

                        // less references wins (e.g. &str over &&str, int over &int)
                        if target_borrows > next_borrows {
                            target = next_type;
                        } else if target_borrows == next_borrows {
                            target = self.unify_types(target, next_type, span, UnifyMode::FindParentType);
                        }
                    }
                }

                // all shall become target.
                for &(expr, _) in &active {
                    let coerced = self.coerce_to_expected_type(expr, target);
                    let expr_span = self.ast.get_expr_span(expr);
                    self.unify_types(target, coerced, expr_span, UnifyMode::Subtype);
                }

                target
            }
        }
    }

    fn get_slice_ref_type(&mut self, mut typ: TypeId) -> Option<TypeId> {
        while let Type::Borrow { inner, mutable, lifetime } = self.prune_type_once(typ) {
            if let Type::Slice(_) = self.prune_type_once(inner) {
                // Return a clean 1-layer reference to the slice: &[T] (or &mut [T])
                return Some(self.type_arena.add_type(Type::Borrow { inner, mutable, lifetime }));
            }
            typ = inner;
        }
        None
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
            | Type::FmtStr
            | Type::Error => Some(true),

            Type::Str
            | Type::Slice(_)
            | Type::Enum(_) => Some(false),

            Type::Tup(TupMode::Tup(inners)) => {
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

            Type::Tup(TupMode::Arr(inner, _))
            | Type::CustomType(_, inner)
            | Type::EnumVariant { inner, .. } => self.is_auto_clone(inner),

            Type::Infer(_) => None
        }
    }

    fn is_unsized(&self, typ: TypeId) -> Option<bool> {
        match self.prune_type_once(typ) {
            Type::Str | Type::Slice(_) => Some(true),
            Type::CustomType(_, inner) => self.is_unsized(inner),

            Type::Infer(_) => None,

            _ => Some(false)
        }
    }
}