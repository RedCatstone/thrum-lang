use std::collections::{HashMap, HashSet};
use crate::{
    ErrType, parsing::ast::{AstTuplePattern, AstValue, Expr, ExprId, Pattern, PatternId}, typing::{Lifetime, TupMode, Type, TypeChecker, TypeId, TypeTuple, TypeVarId, UnifyMode, check_expressions::CheckExprCtx, exhaustiveness::PatternSpace, type_vars::{DefineVarMode, DefineVarScope, TypeVarConstVal}}, vm_compiling::VmValue
};

pub struct CheckPatternCtx<'ast, 'short> {
    expected_type: Option<TypeId>,
    explicit_type_anot: bool,
    has_value: bool,
    can_bind_vars: bool,
    const_update: Option<&'short TypeVarConstVal>,
    vars_defined: &'short mut HashMap<&'ast str, TypeVarId>,
    or_pattern_expects_vars: Option<&'short HashMap<&'ast str, TypeVarId>>,
}
impl<'ast, 'short> CheckPatternCtx<'ast, 'short> {
    pub const fn new(
        expected_type: Option<TypeId>,
        has_value: bool,
        can_bind_vars: bool,
        vars_defined: &'short mut HashMap<&'ast str, TypeVarId>,
        const_update: Option<&'short TypeVarConstVal>
    ) -> Self {
        CheckPatternCtx {
            expected_type, has_value, can_bind_vars, const_update, vars_defined,
            explicit_type_anot: false,
            or_pattern_expects_vars: None
        }
    }

    const fn with_expected(&mut self, expected: Option<TypeId>) -> CheckPatternCtx<'ast, '_> {
        CheckPatternCtx { expected_type: expected, ..self.reborrow() }
    }
    const fn with_explicit_type_anot(&mut self) -> CheckPatternCtx<'ast, '_> {
        CheckPatternCtx { explicit_type_anot: true, ..self.reborrow() }
    }
    const fn with_or_pattern_expects_vars<'r>(&'r mut self, or_pattern_expects_vars: &'r HashMap<&'ast str, TypeVarId>) -> CheckPatternCtx<'ast, 'r> {
        CheckPatternCtx { or_pattern_expects_vars: Some(or_pattern_expects_vars), ..self.reborrow() }
    }
    const fn with_vars_defined<'r>(&'r mut self, vars_defined: &'r mut HashMap<&'ast str, TypeVarId>) -> CheckPatternCtx<'ast, 'r> {
        CheckPatternCtx { vars_defined, ..self.reborrow() }
    }
    /// kind of a hack to get around the borrow checker
    const fn reborrow<'r>(&'r mut self) -> CheckPatternCtx<'ast, 'r> {
        CheckPatternCtx { vars_defined: self.vars_defined, or_pattern_expects_vars: self.or_pattern_expects_vars, ..*self }
    }
}


impl<'ast> TypeChecker<'ast> {
    pub(super) fn check_match_pattern(&mut self, pattern: PatternId, mut ctx: CheckPatternCtx<'ast, '_>) -> (TypeId, Vec<PatternSpace>) {
        let span = self.ast.get_pattern_span(pattern);
        let pattern_pattern = self.ast.get_pattern(pattern);

        let mut covered_cases = Vec::new();

        let inferred_type = match pattern_pattern {
            Pattern::Binding { name, mutable } => {
                // bindings can't fail, so any case is covered here now.
                covered_cases.push(PatternSpace::All);

                let (var_id, typ) = if let Some(or_pattern_expects_vars) = ctx.or_pattern_expects_vars
                    && let Some(var_id) = or_pattern_expects_vars.get(&**name) {
                        (*var_id, self.typed_ast.get_var(*var_id).typ)
                    }
                    else {
                        let typ = ctx.expected_type.unwrap_or_else(|| self.new_infer_type());

                        let var_id = if let Some(const_val) = ctx.const_update {
                            // if we are checking a const pattern, e.g. `const x = 5`
                            // it already knows about `x` and just needs to update it
                            let existing_id = self.typed_ast.resolved_pattern_var[&pattern];
                            let var = self.typed_ast.get_var_mut(existing_id);
                            var.typ = typ;
                            var.const_val = const_val.clone();
                            existing_id
                        } else {
                            self.define_variable(
                                name, typ, ctx.explicit_type_anot, span, DefineVarScope::CurrScope,
                                DefineVarMode::Var { mutable: *mutable, is_init: ctx.has_value }
                            )
                        };
                        (var_id, typ)
                    };

                self.typed_ast.resolved_pattern_var.insert(pattern, var_id);
                let old_var = ctx.vars_defined.insert(name, var_id);
                if old_var.is_some() {
                    self.error(ErrType::TyperPatternNameAlreadyBound { var: self.typed_ast.get_var(var_id).clone() }, span);
                }

                typ
            }

            Pattern::Tuple(elems) => {
                let (pointer_count, base_expected) = self.unwrap_and_record_pattern_derefs(pattern, ctx.expected_type);

                let mut tuple_types = Vec::new();
                let mut tuple_covered_cases = Vec::new();

                let pruned_expected = base_expected.map(|t| self.prune_type_once(t));

                for AstTuplePattern { label, pattern: p } in elems {
                    let elem_expected_type = match &pruned_expected {
                        Some(Type::Tup(tup_mode)) => {

                            tup_mode.extract_label_type(label)
                                // if the pattern was a ref to a tuple, make refs to all elements
                                .map(|(_, typ)| if pointer_count > 0 {
                                        let lifetime = self.new_infer_lifetime();
                                        self.type_arena.add_type(Type::Borrow { inner: typ, mutable: false, lifetime })
                                    } else {
                                        typ
                                    })
                        }
                        _ => None
                    };

                    let (typ, covered) = self.check_match_pattern(*p, ctx.with_expected(elem_expected_type));
                    tuple_types.push(TypeTuple { label: label.clone(), typ });
                    tuple_covered_cases.push(covered);
                }

                covered_cases = PatternSpace::tuple_cartesian_product(&tuple_covered_cases);

                if let Some(base) = base_expected {
                    let elems = tuple_types.iter().map(|tt| {
                            let typ = if let Some((_, _, elem_base)) = self.count_derefable_pointers(tt.typ) {
                                elem_base
                            } else {
                                self.error(ErrType::TyperTypeMustBeKnownHere { typ: self.fmt_type(tt.typ) }, span)
                            };

                            TypeTuple { label: tt.label.clone(), typ }
                        }).collect();

                    let unborrowed_tuple = self.type_arena.add_type(Type::Tup(TupMode::Tup(elems)));
                    self.unify_types(base, unborrowed_tuple, span, UnifyMode::Subtype);
                }

                if pointer_count > 0 {
                    ctx.expected_type.expect("if pointer_count exists, then ctx.expected_type should exist.")
                } else {
                    self.type_arena.add_type(Type::Tup(TupMode::Tup(tuple_types)))
                }
            }

            Pattern::String { before: _, hole_parts } => {
                let str_slice_type = self.type_arena.get_static_str_borrow();

                let (_, base_expected) = self.unwrap_and_record_pattern_derefs(pattern, ctx.expected_type);
                if let Some(base) = base_expected {
                    self.unify_types(base, str_slice_type, span, UnifyMode::Subtype);
                }

                // INVALID: "...{_}{_}..."
                // VALID: "...{_} {_}..."
                // VALID: "...{_}..."
                for (i, (hole_pat, after)) in hole_parts.iter().enumerate() {
                    self.check_match_pattern(*hole_pat, ctx.with_expected(Some(str_slice_type)));

                    if i != hole_parts.len() - 1 && after.is_empty() {
                        self.error(ErrType::TyperPatternStringHolesInARow, span);
                    }
                }
                ctx.expected_type.unwrap_or(str_slice_type)
            }

            Pattern::Wildcard => {
                covered_cases.push(PatternSpace::All);
                self.new_infer_type()
            }

            Pattern::Not(pat) => {
                let mut vars_defined = HashMap::new();
                let (typ, covered) = self.check_match_pattern(*pat, ctx.with_vars_defined(&mut vars_defined));

                if !vars_defined.is_empty() {
                    // it doesn't make sense for a not-pattern to bind vars.
                    // even something double-negated like `... is let !(!x, 5)`
                    // means either bind x OR 2nd is not 5, which doesn't make sense.
                    // and `... is let !!x` can just be simplified to `... is let x`
                    self.error(ErrType::TyperNotPatternCantBindVars, span);
                }

                // invert the covered cases
                covered_cases.extend(PatternSpace::covered_to_missing_cases(&covered, &self.typed_ast.enum_defs));

                typ
            }

            Pattern::Or(inner_patterns) => {
                let (&first_pattern, other_patterns) = inner_patterns.split_first()
                    .expect("Parser makes sure that there are always at least 2 patterns here");

                let mut first_pattern_vars = HashMap::new();
                // check the first pattern normal
                let (mut first_type, covered) = self.check_match_pattern(first_pattern, ctx.with_vars_defined(&mut first_pattern_vars));
                covered_cases.extend(covered);

                for &p in other_patterns {
                    let expected_vars = first_pattern_vars.clone();
                    let mut vars_defined = HashMap::new();
                    let (typ, covered) = self.check_match_pattern(
                        p, ctx.with_or_pattern_expects_vars(&expected_vars).with_vars_defined(&mut vars_defined)
                    );
                    covered_cases.extend(covered);
                    first_type = self.unify_types(first_type, typ, span, UnifyMode::FindParentType);

                    if expected_vars != vars_defined {
                        let expected_set: HashSet<&str> = expected_vars.into_keys().collect();
                        let this_set: HashSet<&str> = vars_defined.into_keys().collect();

                        let didnt_bind: Vec<String> = expected_set.difference(&this_set).map(std::string::ToString::to_string).collect();
                        let too_much: Vec<String> = this_set.difference(&expected_set).map(std::string::ToString::to_string).collect();

                        if !didnt_bind.is_empty() {
                            self.error(ErrType::TyperOrPatternDoesntBindVars { vars: didnt_bind }, span);
                        }
                        if !too_much.is_empty() {
                            self.error(ErrType::TyperOrPatternBindsVarsTooMuch { vars: too_much }, span);
                        }
                    }
                }
                first_type
            }

            Pattern::Conditional { pattern: p, cond } => {
                let (typ, _) = self.check_match_pattern(*p, ctx.reborrow());

                let cond_ctx = CheckExprCtx::default()
                    .expect(TypeId::BOOL)
                    .allow_is_bindings(ctx.can_bind_vars);
                self.check_expression(*cond, &mut false, cond_ctx);

                typ
                // missing cases defaults to NotCovered here, which is correct
            }

            Pattern::TypeDestructor { typ, data } => {
                // basically copy-pasted from check_expressions.rs `Expr::TypeInstantiation`
                let meta_id = self.check_annotation_meta_type_id(*typ, true);
                match self.prune_type_once_infer_err(meta_id, span) {
                    Type::CustomType(_, inner_new_type) => {

                        // because NewType's don't actually exist in the VM, this pattern can keep all pointers
                        // `let NewInt{ inner } = x`  if x was type `&&&NewInt`, inner is type `&&&int`
                        let elem_expected_type = ctx.expected_type.map(|expected| {
                            self.replace_borrow_inner_recursive(expected, inner_new_type)
                        });

                        match self.prune_type_once_infer_err(inner_new_type, span) {
                            Type::Error => TypeId::ERROR,
                            Type::Tup(_) => {
                                // if it expects a tuple, e.g. `type Point = { num, num }`
                                // then just typecheck normally. (`data` is already a tuple expr)
                                let (typ, covered) = self.check_match_pattern(
                                    *data, ctx.with_expected(elem_expected_type)
                                );
                                covered_cases = covered;
                                typ
                            }
                            _ => {
                                // if it doesn't expect a tuple, it needs to extract the first element.
                                let Pattern::Tuple(elems) = self.ast.get_pattern(*data) else {
                                    unreachable!("this is always a tuple.")
                                };
                                if let [first] = elems.as_slice() && first.label == "0" {
                                    self.typed_ast.resolved_type_destruction_not_a_tuple.insert(pattern);
                                    let (typ, covered) = self.check_match_pattern(
                                        first.pattern, ctx.with_expected(elem_expected_type)
                                    );
                                    covered_cases = covered;
                                    typ
                                }
                                else {
                                    self.error(ErrType::TyperNewTypesExpectOneUnlabeledExpr, span)
                                }
                            }
                        };

                        // `N{ 2 }` returns the type `N`
                        ctx.expected_type.unwrap_or(meta_id)
                    }
                    Type::Error => TypeId::ERROR,
                    _ => self.error(ErrType::TyperMustBeCustomtypeType { typ: self.fmt_type(meta_id) }, self.ast.get_expr_span(*typ))
                }
            }

            Pattern::EnumVariant { name, attached_tuple } => {
                let (pointer_count, base_expected) = self.unwrap_and_record_pattern_derefs(pattern, ctx.expected_type);

                let mut expected_refined_enum = false;

                // using `.Variant` syntax requires that the Typechecker knows the Enumtype.
                let resolved_variant =
                    if let Some(expected) = base_expected
                    && let Type::EnumVariant { inner, variant } = self.prune_type_once(expected) {
                        // its a hard refined enum
                        // e.g. `let .Some{ inner } = Option.Some{ 123 }`
                        expected_refined_enum = true;

                        let enum_id = self.get_wrapped_enum_id(inner).unwrap();
                        let variants = &self.typed_ast.enum_defs[enum_id.0 as usize].variants;

                        let (variant_name, attached_type) = variants[variant].clone();
                        if *variant_name != *name {
                            self.error(ErrType::TyperEnumExpectedExactVariant { variant: variant_name.to_string(), found: name.clone() }, span);
                        }

                        Some((enum_id, variant, attached_type, expected))
                    }
                    else {
                        self.check_enum_variant(name, base_expected, Some(span))
                    };


                if let Some((enum_id, variant_index, attached_type, final_type)) = resolved_variant {
                    // now that we found a variant, handle the inner data

                    let inner_covered = if let Some(tup) = attached_tuple {
                        let elem_expected_type = if pointer_count > 0 {
                            let lifetime = self.new_infer_lifetime();
                            self.type_arena.add_type(Type::Borrow { inner: attached_type, mutable: false, lifetime })
                        } else {
                            attached_type
                        };
                        self.check_match_pattern(*tup, ctx.with_expected(Some(elem_expected_type))).1
                    } else {
                        self.unify_types(TypeId::VOID, attached_type, span, UnifyMode::Subtype);
                        vec![PatternSpace::All]
                    };
                    for covered in inner_covered {
                        if expected_refined_enum {
                            covered_cases.push(covered);
                        } else {
                            covered_cases.push(PatternSpace::EnumVariant { enum_id, variant_index, attached_tuple: Box::new(covered) });
                        }
                    }
                    self.typed_ast.resolved_enum_variant_pattern.insert(pattern,(enum_id,variant_index));

                    if pointer_count > 0 {
                        ctx.expected_type.unwrap()
                    } else {
                        final_type
                    }
                } else {
                    TypeId::ERROR
                }
            }

            Pattern::CompareExpr(expr) => {
                let (_, base_expected) = self.unwrap_and_record_pattern_derefs(pattern, ctx.expected_type);

                let expr_type = self.check_expression(*expr, &mut false, CheckExprCtx::default().maybe_expect(base_expected));

                // if there the compare expr is a simple literal, or a const value
                // then add that to covered_cases
                match self.ast.get_expr(*expr) {
                    Expr::Literal { val: AstValue::Bool(b) } => covered_cases.push(PatternSpace::Bool(*b)),
                    Expr::IdentifierRef { .. } => {
                        if let Some(var_id) = self.typed_ast.resolved_expr_var.get(expr)
                        && let TypeVarConstVal::Evaluated(val) = &self.typed_ast.get_var(*var_id).const_val {
                            #[allow(clippy::collapsible_match, clippy::single_match)]
                            match val {
                                VmValue::Bool(b) => covered_cases.push(PatternSpace::Bool(*b)),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }

                ctx.expected_type.unwrap_or(expr_type)
            }

            Pattern::PlacePointer(expr) => {
                // PlacePointer can never fail, so any case is covered here now.
                covered_cases.push(PatternSpace::All);

                // expr was already typechecked in `get_pattern_type`
                let expr_type = self.typed_ast.get_expr_type(*expr);
                match self.prune_type_once(expr_type) {
                    Type::Borrow { inner, mutable: true, lifetime } => {
                        if let Lifetime::LocalVariable(x) = lifetime {
                            self.update_variable(x, span);
                        }
                        inner
                    }
                    _ => TypeId::ERROR
                }
            }

            // we are actually gonna ignore this type annotation.
            // in let bindings: e.g. `let x: u32 = 5`
            // the type is first checked
            // then the expr with that type
            // then the full pattern with that type, so no need here
            Pattern::Typed { pattern, typ: _ } => {
                let (pattern_typ, covered) = self.check_match_pattern(*pattern, ctx.with_explicit_type_anot());
                covered_cases.extend(covered);
                // self.unify_types(typ, other, span);
                pattern_typ
            }
        };

        if let Some(expected) = ctx.expected_type {
            self.unify_types(expected, inferred_type, span, UnifyMode::Subtype);
        }

        (inferred_type, covered_cases)
    }





    fn unwrap_and_record_pattern_derefs(&mut self, pattern: PatternId, expected: Option<TypeId>) -> (usize, Option<TypeId>) {
        let (pointer_count, base_expected) = match expected.and_then(|e| self.count_derefable_pointers(e)) {
            Some((count, _, base)) => (count, Some(base)),
            None => (0, expected),
        };
        self.typed_ast.resolved_pattern_derefs.insert(pattern, pointer_count);
        (pointer_count, base_expected)
    }


    pub(super) fn check_annotation_meta_type_id(&mut self, expr: ExprId, needs_typechecking: bool) -> TypeId {
        if needs_typechecking {
            self.check_expression(expr, &mut false, CheckExprCtx::default().expect(TypeId::TYPE).is_const());
        }

        self.evaluate_expr(expr).map_or(TypeId::ERROR, |val| {
            let VmValue::Type(meta_id) = val else {
                unreachable!("not a meta type?! {val}, expr: {}", self.ast.display_expr(expr))
            };
            meta_id
        })
    }


    pub(super) fn get_pattern_type(&mut self, pattern: PatternId) -> Option<TypeId> {
        match self.ast.get_pattern(pattern) {
            Pattern::Typed { typ, .. } => Some(
                self.check_annotation_meta_type_id(*typ, true)
            ),
            Pattern::PlacePointer(expr) => {
                let expr_type = self.check_expression(*expr, &mut false, CheckExprCtx::default().auto_borrow_mut(true));
                let span = self.ast.get_expr_span(*expr);

                Some(match self.prune_type_once_infer_err(expr_type, span) {
                    Type::Borrow { inner, mutable: true, lifetime: _ } => {
                        inner
                    }
                    Type::Error => TypeId::ERROR,
                    _ => {
                        let err_borrow = self.type_arena.add_type(Type::Borrow { inner: TypeId::ERROR, mutable: true, lifetime: Lifetime::Static });
                        self.type_mismatch(err_borrow, expr_type, span)
                    }
                })
            }
            Pattern::Tuple(elems) => {
                let mut any_explicit = false;
                let type_elems = elems.iter()
                    .map(|elem| {
                        let elem_type = self.get_pattern_type(elem.pattern).map_or_else(|| self.new_infer_type(), |x| {
                                any_explicit = true;
                                x
                            });
                        TypeTuple { label: elem.label.clone(), typ: elem_type }
                    })
                    .collect();

                // this is not a perfect change, but it works for now
                if any_explicit {
                    Some(self.type_arena.add_type(Type::Tup(TupMode::Tup(type_elems))))
                } else {
                    None
                }
            }
            _ => None,
        }
    }


    pub(super) fn mark_vars_in_pattern_as_const(&mut self, pattern: PatternId, const_val: TypeVarConstVal, define_scope: DefineVarScope) {
        let span = self.ast.get_pattern_span(pattern);
        match self.ast.get_pattern(pattern) {
            Pattern::Binding { name, mutable } => {
                if let Some(var_id) = self.typed_ast.resolved_pattern_var.get(&pattern) {
                    // pattern var already exists!
                    self.typed_ast.get_var_mut(*var_id).const_val = const_val;
                } else {
                    // pattern var doesn't exist so make one
                    let var_id = self.define_variable(
                        name, TypeId::VOID, true, span, define_scope, DefineVarMode::Const(const_val)
                    );
                    self.typed_ast.resolved_pattern_var.insert(pattern, var_id);

                    if *mutable {
                        self.error(ErrType::TyperConstCantBeMutable, span);
                    }
                }
            },
            Pattern::Or(patterns) => {
                // all vars in an or-pattern must be the same, so only marking the first one is fine
                self.mark_vars_in_pattern_as_const(patterns[0], const_val, define_scope);
            }
            Pattern::Tuple(ast_tuple_patterns) => {
                // if we have a const_val, then we need to destructure the const tuple to each pattern.
                if let TypeVarConstVal::Evaluated(VmValue::Tup(elems)) = const_val {
                    assert_eq!(elems.len(), ast_tuple_patterns.len());

                    for (p, elem) in ast_tuple_patterns.iter().zip(elems) {
                        self.mark_vars_in_pattern_as_const(p.pattern, TypeVarConstVal::Evaluated(elem), define_scope);
                    }
                }
                else {
                    for p in ast_tuple_patterns {
                        self.mark_vars_in_pattern_as_const(p.pattern, const_val.clone(), define_scope);
                    }
                }
            }
            Pattern::EnumVariant { name: _, attached_tuple } => {
                if let Some(tup) = attached_tuple {
                    self.mark_vars_in_pattern_as_const(*tup, const_val, define_scope);
                }
            }
            Pattern::String { before: _, hole_parts } => {
                for hole_part in hole_parts {
                    self.mark_vars_in_pattern_as_const(hole_part.0, const_val.clone(), define_scope);
                }
            }
            Pattern::Conditional { pattern, cond: _ }
            | Pattern::Typed { pattern, typ: _ }
            | Pattern::TypeDestructor { typ: _, data: pattern }
            | Pattern::Not(pattern) => self.mark_vars_in_pattern_as_const(*pattern, const_val, define_scope),

            Pattern::Wildcard | Pattern::CompareExpr(_) | Pattern::PlacePointer(_) => { /* no vars */ },
        }
    }
}