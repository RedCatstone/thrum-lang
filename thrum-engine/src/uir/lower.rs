use std::collections::HashMap;

use crate::{
    parsing::ast::{AstArena, BinOp, Expr, ExprId, Pattern, PatternId}, typing::{ResolvedMemberAccess, ResolvedTypeInstantiation, TupMode, Type, TypeArena, TypeId, TypeVarId, TypedAst}, uir::ir::{
        BlockId, Constant, FunctionId, LocalId, Operand, RValue, Statement, Terminator, UirFunction,
    }, vm_compiling::NumMode,
};

pub struct FnLowerer<'a> {
    ast: &'a AstArena,
    typed_ast: &'a TypedAst,
    type_arena: &'a TypeArena,
    function: UirFunction,
    current_block: BlockId,
    var_to_local: HashMap<TypeVarId, LocalId>,
    loop_infos: HashMap<ExprId, (BlockId, BlockId, LocalId)>,
}

impl<'a> FnLowerer<'a> {
    #[must_use] pub fn start(ast: &'a AstArena, typed_ast: &'a TypedAst, type_arena: &'a TypeArena) -> UirFunction {
        let mut lowerer = Self::new(ast, typed_ast, type_arena);
        lowerer.lower_function_root(ExprId(0));
        lowerer.function
    }

    fn new(ast: &'a AstArena, typed_ast: &'a TypedAst, type_arena: &'a TypeArena) -> Self {
        let return_type = typed_ast.get_expr_type(ExprId(0));
        let return_size = type_arena.type_size(return_type);
        let function = UirFunction::new(FunctionId(0), "main".to_string(), 0, return_type, return_size);
        let current_block = function.entry_block;

        Self { ast, typed_ast, type_arena, function, current_block, var_to_local: HashMap::new(), loop_infos: HashMap::new() }
    }

    fn lower_function_root(&mut self, root: ExprId) {
        self.lower_expr_to_local(root, LocalId::RETURN_PLACE);
        self.set_curr_terminator(Terminator::Return);
    }

    /// lowers the expression so it directly ends up in `dest`.
    fn lower_expr_to_local(&mut self, expr_id: ExprId, dest: LocalId) {
        let deref_count = self.typed_ast.auto_derefs.get(&expr_id);
        let temp_dest = if deref_count.is_some() {
            self.function.new_temp(TypeId::INT, 8)
        } else {
            dest
        };

        self.lower_expr_to_local_internal(expr_id, temp_dest);

        // auto-derefs from the typechecker
        if let Some(&derefs) = deref_count {
            let mut curr = temp_dest;
            for i in 0..derefs {
                let target = if i + 1 == derefs { dest } else { self.function.new_temp(TypeId::INT, 8) };
                self.add_curr_statement(Statement::Assign(target, RValue::Deref(curr)));
                curr = target;
            }
        }
    }


    fn lower_expr_to_local_internal(&mut self, expr_id: ExprId, dest: LocalId) {
        match self.ast.get_expr(expr_id) {
            Expr::Literal { val } => {
                let constant = Constant::from_val(val);
                self.add_curr_statement(Statement::Assign(dest, RValue::Use(Operand::Constant(constant))));
            }

            Expr::Infix { op, left, right, op_span: _ } => {
                let left_op = self.lower_expr_to_operand(*left);
                let right_op = self.lower_expr_to_operand(*right);
                let num_mode = self.typed_ast.resolved_num_mode.get(&expr_id).copied().unwrap_or(NumMode::Int);

                self.add_curr_statement(Statement::Assign(dest, RValue::BinaryOp(*op, num_mode, left_op, right_op)));
            }

            Expr::Prefix { op, right } => {
                let right_op = self.lower_expr_to_operand(*right);
                let num_mode = self.typed_ast.resolved_num_mode[&expr_id];

                self.add_curr_statement(Statement::Assign(dest, RValue::UnaryOp(*op, num_mode, right_op)));
            }

            Expr::Block { exprs, label: _ } => {
                if let Some((last, others)) = exprs.split_last() {
                    for expr in others {
                        let _discard_temp = self.new_temp_expr(*expr);

                        // if an expression returns never, then it doesn't need to compile the exprs after
                        if let TypeId::NEVER = self.typed_ast.get_expr_type(*expr) {
                            return
                        }
                    }
                    self.lower_expr_to_local(*last, dest);
                }
            }

            Expr::Assign { pattern, value, extra_op, op_span: _ } => {
                if let Some(extra_op) = extra_op {
                    // example: x += 2
                    // it needs to first deref x, evaluate x + 2, then set x to that result
                    let Pattern::PlacePointer(target_expr) = self.ast.get_pattern(*pattern) else {
                        unreachable!("Infix assignments should only be possible on place patterns.")
                    };

                    let ptr_local = self.new_temp_expr(*target_expr);
                    let target_borrow_type = self.typed_ast.get_expr_type(*target_expr);
                    let Type::Borrow { inner: target_val_type, .. } = self.type_arena.get_type(target_borrow_type) else {
                        unreachable!("should be a pointer type...")
                    };

                    let dereffed_ptr_local = self.new_temp_of_type(target_val_type);
                    self.add_curr_statement(Statement::Assign(dereffed_ptr_local, RValue::Deref(ptr_local)));

                    let right_op = self.lower_expr_to_operand(*value);
                    let num_mode = self.typed_ast.resolved_num_mode[&expr_id];

                    let result_local = self.new_temp_of_type(target_val_type);
                    self.add_curr_statement(Statement::Assign(
                        result_local,
                        RValue::BinaryOp(*extra_op, num_mode, Operand::Copy(dereffed_ptr_local), right_op),
                    ));
                    self.add_curr_statement(Statement::Store {
                        pointer: ptr_local,
                        value: Operand::Copy(result_local),
                    });
                }
                else {
                    let val_place = self.new_temp_expr(*value);
                    self.lower_pattern(*pattern, val_place, None);
                }
            }

            Expr::EmptyLet { pattern } => {
                let var_id = self.typed_ast.resolved_pattern_var[pattern];
                self.get_or_create_local(var_id);
            }

            Expr::IdentifierRef { .. } => {
                let var_id = self.typed_ast.resolved_expr_var[&expr_id];
                let var = self.typed_ast.get_var(var_id);

                let local = self.var_to_local[&var_id];

                self.add_curr_statement(Statement::Assign(dest, RValue::MakeRef { mutable: var.is_declared_mut, local }));
            }

            Expr::Move { expr } => {
                let expr_local = self.new_temp_expr(*expr);
                self.add_curr_statement(Statement::Assign(dest, RValue::Deref(expr_local)));
            }


            Expr::Tuple { elems } => {
                let ops = elems
                    .iter()
                    .map(|e| self.lower_expr_to_operand(e.expr))
                    .collect();
                self.add_curr_statement(Statement::Assign(dest, RValue::MakeTuple(ops)));
            }

            Expr::TupleArr { elem, .. } => {
                let length = self.typed_ast.resolved_tuple_arr_length[&expr_id];
                let elem_op = self.lower_expr_to_operand(*elem);
                self.add_curr_statement(Statement::Assign(
                    dest,
                    RValue::MakeTupleArr { elem: elem_op, length },
                ));
            }

            Expr::Index { left, index } => {
                let left_local = self.new_temp_expr(*left);
                let index_op = self.lower_expr_to_operand(*index);
                let left_type = self.typed_ast.get_expr_type(*left);
                let (elem_size, len) = self.get_array_elem_size_and_len(left_type);

                self.add_curr_statement(Statement::Assign(
                    dest,
                    if self.typed_ast.resolved_index_type_is_tup.contains(&expr_id) {
                        RValue::IndexRef { base: left_local, index: index_op, elem_size, len }  // TODO: fix
                    } else {
                        todo!()
                    }
                ));
            }

            Expr::MemberAccess { left, .. } => {
                match &self.typed_ast.resolved_member_access[&expr_id] {
                    ResolvedMemberAccess::TupleIndex { index } => {
                        let left_local = self.new_temp_expr(*left);
                        let tup_type = self.typed_ast.get_expr_type(*left);
                        let offset = self.type_arena.tuple_field_offset(tup_type, *index);

                        self.add_curr_statement(Statement::Assign(
                            dest,
                            RValue::TupleGet { tuple: left_local, offset },
                        ));
                    }
                    ResolvedMemberAccess::TupleRefIndex { index } => {
                        // TODO: fix pointer thingie
                        let left_local = self.new_temp_expr(*left);
                        let tup_type = self.typed_ast.get_expr_type(*left);
                        let offset = self.type_arena.tuple_field_offset(tup_type, *index);

                        self.add_curr_statement(Statement::Assign(
                            dest,
                            RValue::TupleGetRef { tuple: left_local, offset },
                        ));
                    }
                    ResolvedMemberAccess::EnumWithNoData { i, enum_id } => {
                        self.add_curr_statement(Statement::Assign(
                            dest,
                            RValue::MakeEnum { enum_id: *enum_id, variant_index: *i, payload: Operand::Constant(Constant::Void) }
                        ));
                    }
                    x => todo!("{x:?} not yet implemented"),
                }
            }

            Expr::EnumVariant { data } => {
                let (enum_id, variant_index) = self.typed_ast.resolved_enum_variant[&expr_id];
                let payload_op = data.attached_tuple.map_or(
                    Operand::Constant(Constant::Void),
                    |tup_expr| self.lower_expr_to_operand(tup_expr)
                );

                self.add_curr_statement(Statement::Assign(
                    dest,
                    RValue::MakeEnum { enum_id, variant_index, payload: payload_op },
                ));
            }

            Expr::TypeInstantiation { typ: _, data } => {
                match self.typed_ast.resolved_type_instantian[&expr_id] {
                    ResolvedTypeInstantiation::NewType => {
                        let Expr::Tuple { elems } = self.ast.get_expr(*data) else {
                            unreachable!("always a tuple.")
                        };
                        self.lower_expr_to_local(elems[0].expr, dest);
                    }
                    ResolvedTypeInstantiation::Tuple => {
                        self.lower_expr_to_local(*data, dest);
                    }
                    ResolvedTypeInstantiation::EnumVariant(variant_index, enum_id) => {
                        let payload_op = self.lower_expr_to_operand(*data);
                        self.add_curr_statement(Statement::Assign(
                            dest,
                            RValue::MakeEnum { enum_id, variant_index, payload: payload_op }
                        ));
                    }
                }
            }


            // Control Flow
            Expr::If { condition, then, alt, never_alt: _ } => {
                let condition_op = self.lower_expr_to_operand(*condition);

                let then_block = self.function.new_block("if_then");
                let alt_block = self.function.new_block("if_alt");
                let after_if_block = self.function.new_block("after_if");

                self.set_curr_terminator(Terminator::MatchOnInt {
                    discr: condition_op,
                    targets: vec![(1, then_block)],
                    otherwise: alt_block
                });

                self.current_block = then_block;
                self.lower_expr_to_local(*then, dest);
                self.set_curr_terminator(Terminator::Goto(after_if_block));

                self.current_block = alt_block;
                self.lower_expr_to_local(*alt, dest);
                self.set_curr_terminator(Terminator::Goto(after_if_block));

                self.current_block = after_if_block;
            }

            Expr::Loop { body, label: _ } => {
                let loop_body = self.function.new_block("loop_body");
                let after_loop = self.function.new_block("after_loop");

                self.loop_infos.insert(expr_id, (loop_body, after_loop, dest));

                self.set_curr_terminator(Terminator::Goto(loop_body));
                self.current_block = loop_body;
                self.lower_expr_to_operand(*body);
                self.set_curr_terminator(Terminator::Goto(loop_body));

                assert_eq!(self.loop_infos.remove(&expr_id), Some((loop_body, after_loop, dest)));

                self.current_block = after_loop;
            }

            Expr::Break { label: _, expr } => {
                let break_to = self.typed_ast.resolved_labels[&expr_id];
                let (_, after_loop, break_dest) = self.loop_infos[&break_to];

                self.lower_expr_to_local(*expr, break_dest);

                self.set_curr_terminator(Terminator::Goto(after_loop));
            }
            Expr::Continue { label: _ } => {
                let break_to = self.typed_ast.resolved_labels[&expr_id];
                let (loop_body, _, _) = self.loop_infos[&break_to];

                self.set_curr_terminator(Terminator::Goto(loop_body));
            }

            Expr::Match { match_value, arms } => {
                let value_local = self.new_temp_expr(*match_value);
                let after_match_block = self.function.new_block("after_match");

                for (i, arm) in arms.iter().enumerate() {
                    let isnt_last = i + 1 != arms.len();
                    let target_blocks = if isnt_last {
                        Some((self.function.new_block("arm_success"), self.function.new_block("arm_fail")))
                    } else {
                        None
                    };

                    self.lower_pattern(arm.pattern, value_local, target_blocks);

                    if let Some((arm_body_block, _)) = target_blocks {
                        self.current_block = arm_body_block;
                    }
                    self.lower_expr_to_local(arm.body, dest);
                    self.set_curr_terminator(Terminator::Goto(after_match_block));

                    // Advance to test the next arm (if there is one)
                    if let Some((_, next_arm_block)) = target_blocks {
                        self.current_block = next_arm_block;
                    }
                }

                self.current_block = after_match_block;
            }

            Expr::Is { value, pattern } => {
                let val_local = self.new_temp_expr(*value);

                let success_block = self.function.new_block("is_success");
                let failure_block = self.function.new_block("is_fail");
                let after_is_block = self.function.new_block("after_is");

                self.lower_pattern(*pattern, val_local, Some((success_block, failure_block)));

                // Matched!
                self.current_block = success_block;
                self.add_curr_statement(Statement::Assign(
                    dest,
                    RValue::Use(Operand::Constant(Constant::Bool(true))),
                ));
                self.set_curr_terminator(Terminator::Goto(after_is_block));

                // Failed!
                self.current_block = failure_block;
                self.add_curr_statement(Statement::Assign(
                    dest,
                    RValue::Use(Operand::Constant(Constant::Bool(false))),
                ));
                self.set_curr_terminator(Terminator::Goto(after_is_block));

                self.current_block = after_is_block;
            }

            // do nothing here, these are void or only for the typechecker.
            Expr::Void
            | Expr::Const { .. }
            | Expr::CustomType { .. }
            | Expr::ImplBlock { .. } => { }

            other => todo!("support for {other:?} not yet implemented"),
        }
    }


    fn lower_expr_to_operand(&mut self, expr_id: ExprId) -> Operand {
        if let Expr::Literal { val } = self.ast.get_expr(expr_id) {
            // literals become immidiate constants
            Operand::Constant(Constant::from_val(val))
        }
        else {
            // everything else gets a quick temporary value instead
            let place = self.new_temp_expr(expr_id);
            Operand::Move(place)
        }
    }

    fn new_temp_expr(&mut self, expr: ExprId) -> LocalId {
        let typ = self.typed_ast.get_expr_type(expr);
        let temp = self.new_temp_of_type(typ);
        self.lower_expr_to_local(expr, temp);
        temp
    }

    fn new_temp_of_type(&mut self, typ: TypeId) -> LocalId {
        let size = self.type_arena.type_size(typ);
        self.function.new_temp(typ, size)
    }

    fn get_or_create_local(&mut self, var_id: TypeVarId) -> LocalId {
        if let Some(&l) = self.var_to_local.get(&var_id) {
            l
        } else {
            let var = self.typed_ast.get_var(var_id);
            let size = self.type_arena.type_size(var.typ);
            let l = self.function.new_local(
                var.typ,
                size,
                Some(var.name.clone().into_boxed_str()),
                var.is_declared_mut,
            );
            self.var_to_local.insert(var_id, l);
            l
        }
    }

    fn get_array_elem_size_and_len(&self, typ: TypeId) -> (usize, usize) {
        let mut curr = typ;
        while let Type::Borrow { inner, .. } = self.type_arena.get_type(curr) {
            curr = inner;
        }
        match self.type_arena.get_type(curr) {
            Type::Tup(TupMode::Arr(elem, len)) => (self.type_arena.type_size(elem), len),
            Type::Tup(TupMode::Tup(elems)) => {
                let elem_size = elems
                    .first()
                    .map_or(8, |e| self.type_arena.type_size(e.typ));
                (elem_size, elems.len())
            }
            Type::CustomType(_, inner) => self.get_array_elem_size_and_len(inner),
            _ => unreachable!(),
        }
    }

    fn add_curr_statement(&mut self, stmt: Statement) {
        self.function.add_statement(self.current_block, stmt);
    }
    fn set_curr_terminator(&mut self, term: Terminator) {
        self.function.set_terminator(self.current_block, term);
    }


    fn num_mode_for_type(&self, typ: TypeId) -> NumMode {
        match self.type_arena.get_type(typ) {
            Type::NumFloat => NumMode::Float,
            Type::CustomType(_, inner) | Type::Borrow { inner, .. } => self.num_mode_for_type(inner),
            _ => NumMode::Int,
        }
    }



    fn lower_pattern(&mut self, pattern: PatternId, val_local: LocalId, success_fail_block: Option<(BlockId, BlockId)>) {
        let pointer_count = self.typed_ast.resolved_pattern_derefs.get(&pattern).copied().unwrap_or(0);
        let mut current_val = val_local;
        let mut current_type = self.function.locals[val_local.0 as usize].type_id;

        for _ in 0..pointer_count {
            let Type::Borrow { inner, .. } = self.type_arena.get_type(current_type) else {
                unreachable!("should be a pointer type...")
            };
            let deref_local = self.new_temp_of_type(inner);
            self.add_curr_statement(Statement::Assign(deref_local, RValue::Deref(current_val)));
            current_val = deref_local;
            current_type = inner;
        }

        let new_val_local = current_val;

        match self.ast.get_pattern(pattern) {
            Pattern::Binding { .. } => {
                let var_id = self.typed_ast.resolved_pattern_var[&pattern];
                let local = self.get_or_create_local(var_id);

                self.add_curr_statement(Statement::Assign(local, RValue::Use(Operand::Copy(new_val_local))));
            }

            Pattern::Wildcard => {
                // Wildcard go brrrr
            }

            Pattern::CompareExpr(expr) => {
                if let Some((success_block, fail_block)) = success_fail_block {
                    let expr_op = self.lower_expr_to_operand(*expr);
                    let val_type = self.function.locals[current_val.0 as usize].type_id;
                    let num_mode = self.num_mode_for_type(val_type);

                    let cmp_temp = self.new_temp_of_type(TypeId::BOOL);
                    self.add_curr_statement(Statement::Assign(
                        cmp_temp,
                        RValue::BinaryOp(BinOp::Equal, num_mode, Operand::Copy(current_val), expr_op),
                    ));

                    self.set_curr_terminator(Terminator::MatchOnInt {
                        discr: Operand::Copy(cmp_temp),
                        targets: vec![(1, success_block)],
                        otherwise: fail_block,
                    });
                } else {
                    // if the pattern is expected to not fail, it doesn't need to compile anything here
                    // example: `let true | false = true`
                }
            }

            Pattern::Or(patterns) => {
                let success_block = match success_fail_block {
                    Some((s, _)) => s,
                    None => self.function.new_block("or_full_success")
                };

                for (i, &pat) in patterns.iter().enumerate() {
                    let is_last = i + 1 == patterns.len();

                    let target_blocks = if is_last {
                        match success_fail_block {
                            Some((s, f)) => Some((s, f)),
                            None => {
                                self.set_curr_terminator(Terminator::Goto(success_block));
                                None
                            }
                        }
                    } else {
                        Some((success_block, self.function.new_block("pattern_or_next")))
                    };

                    self.lower_pattern(pat, current_val, target_blocks);

                    if let Some((_, fail_next_block)) = target_blocks {
                        self.current_block = fail_next_block;
                    }
                }

                self.current_block = success_block;
            }

            Pattern::Conditional { pattern, cond } => {
                let (success_block, fail_block) = success_fail_block.unwrap();
                let cond_block = self.function.new_block("cond_pattern");

                self.lower_pattern(*pattern, new_val_local, Some((cond_block, fail_block)));

                self.current_block = cond_block;
                let cond_op = self.lower_expr_to_operand(*cond);

                self.set_curr_terminator(Terminator::MatchOnInt {
                    discr: cond_op,
                    targets: vec![(1, success_block)],
                    otherwise: fail_block,
                });
            }

            Pattern::Not(pattern) => {
                self.lower_pattern(*pattern, new_val_local, success_fail_block.map(|(s, f)| (f, s)));
            }

            Pattern::Tuple(elems) => {
                let val_local_type = self.function.locals[new_val_local.0 as usize].type_id;
                let mut offset = 0;

                for (i, elem) in elems.iter().enumerate() {
                    let elem_type = self.type_arena.tuple_field_type(val_local_type, i);
                    let elem_size = self.type_arena.type_size(elem_type);
                    let elem_local = self.new_temp_of_type(elem_type);

                    self.add_curr_statement(Statement::Assign(
                        elem_local,
                        RValue::TupleGet { tuple: new_val_local, offset },
                    ));
                    offset += elem_size;

                    let is_last = i + 1 == elems.len();

                    // If we have a failure block, each element must jump to the next element block
                    // or to the failure block if it doesn't match.
                    let elem_targets = match success_fail_block {
                        Some((success_block, fail_block)) => {
                            let next_elem_block = if is_last { success_block } else { self.function.new_block("pattern_tup_next") };
                            Some((next_elem_block, fail_block))
                        }
                        None => None  // no blocks added if no branches can fail!
                    };

                    self.lower_pattern(elem.pattern, elem_local, elem_targets);

                    if let Some((next_elem_block, _)) = elem_targets
                    && !is_last {
                        self.current_block = next_elem_block;
                    }
                }
            }

            Pattern::EnumVariant { name: _, attached_tuple } => {
                let (enum_id, variant_index) = self.typed_ast.resolved_enum_variant_pattern[&pattern];

                if let Some((_, fail_block)) = success_fail_block {
                    let discr_temp = self.new_temp_of_type(TypeId::INT);
                    self.add_curr_statement(Statement::Assign(discr_temp, RValue::ReadDiscriminant(current_val)));

                    let payload_check_block = self.function.new_block("pattern_enum_payload");

                    self.set_curr_terminator(Terminator::MatchOnInt {
                        discr: Operand::Copy(discr_temp),
                        targets: vec![(variant_index.try_into().unwrap(), payload_check_block)],
                        otherwise: fail_block,
                    });

                    self.current_block = payload_check_block;
                } else {
                    // if its expected to not fail, then we also don't need to check the discriminant ;p
                }

                if let Some(tup_pat) = attached_tuple {
                    let payload_type = self.type_arena.enum_defs[enum_id.0 as usize].variants[variant_index].1;

                    let payload_local = self.new_temp_of_type(payload_type);
                    self.add_curr_statement(Statement::Assign(
                        payload_local,
                        RValue::TupleGet { tuple: val_local, offset: 8 },
                    ));

                    self.lower_pattern(*tup_pat, payload_local, success_fail_block);
                }
            }

            Pattern::PlacePointer(expr) => {
                let ptr_local = self.new_temp_expr(*expr);
                self.add_curr_statement(Statement::Store {
                    pointer: ptr_local,
                    value: Operand::Copy(new_val_local),
                });
            }

            Pattern::Typed { pattern, typ: _ } => {
                self.lower_pattern(*pattern, val_local, success_fail_block);
            }

            x => todo!("{x:?} not yet implemented...")
        }

        // if the block terminator wasn't set yet, just set it to jump to success_block
        if let Some((success_block, _)) = success_fail_block {
            self.set_curr_terminator(Terminator::Goto(success_block));
        }
    }
}


impl TypeArena {
    #[must_use] pub fn tuple_field_offset(&self, tup_typ: TypeId, field_idx: usize) -> usize {
        match self.get_type(tup_typ) {
            Type::Tup(TupMode::Tup(elems)) => elems
                .iter()
                .take(field_idx)
                .map(|e| self.type_size(e.typ))
                .sum(),
            Type::Tup(TupMode::Arr(elem, _)) => field_idx * self.type_size(elem),
            Type::CustomType(_, inner)
            | Type::Borrow { inner, .. } => self.tuple_field_offset(inner, field_idx),
            _ => unreachable!(),
        }
    }

    #[must_use] pub fn tuple_field_type(&self, tup_typ: TypeId, field_idx: usize) -> TypeId {
        match self.get_type(tup_typ) {
            Type::Tup(TupMode::Tup(elems)) => elems[field_idx].typ,
            Type::Tup(TupMode::Arr(elem, _)) => elem,
            Type::CustomType(_, inner)
            | Type::Borrow { inner, .. } => self.tuple_field_type(inner, field_idx),
            _ => unreachable!(),
        }
    }
}