use std::collections::HashMap;

use crate::{
    parsing::ast::{AstArena, Expr, ExprId, Pattern, PatternId}, typing::{ResolvedMemberAccess, TupMode, Type, TypeArena, TypeId, TypeVarId, TypedAst}, uir::ir::{
        BlockId, Constant, FunctionId, LocalId, Operand, RValue, Statement, Terminator, UirFunction,
    },
};

pub struct FnLowerer<'a> {
    ast: &'a AstArena,
    typed_ast: &'a TypedAst,
    type_arena: &'a TypeArena,
    function: UirFunction,
    current_block: BlockId,
    var_to_local: HashMap<TypeVarId, LocalId>,
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

        Self { ast, typed_ast, type_arena, function, current_block, var_to_local: HashMap::new() }
    }

    fn lower_function_root(&mut self, root: ExprId) {
        self.lower_expr_to_local(root, LocalId::RETURN_PLACE);
        self.function.set_terminator(self.current_block, Terminator::Return);
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
            let mut curr = dest;
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
            Expr::Void => {
                let constant = Constant::Void;
                self.add_curr_statement(Statement::Assign(dest, RValue::Use(Operand::Constant(constant))));
            }

            Expr::Infix { op, left, right, op_span: _ } => {
                let left_op = self.lower_expr_to_operand(*left);
                let right_op = self.lower_expr_to_operand(*right);
                let num_mode = self.typed_ast.resolved_num_mode[&expr_id];

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
                    }
                    self.lower_expr_to_local(*last, dest);
                }
                else {
                    self.add_curr_statement(Statement::Assign(
                        dest,
                        RValue::Use(Operand::Constant(Constant::Void)),
                    ));
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
                    self.lower_pattern(*pattern, val_place);
                }

                self.add_curr_statement(Statement::Assign(
                    dest,
                    RValue::Use(Operand::Constant(Constant::Void)),
                ));
            }

            Expr::EmptyLet { pattern } => {
                let var_id = self.typed_ast.resolved_pattern_var[pattern];
                self.get_or_create_local(var_id);
                self.add_curr_statement(Statement::Assign(
                    dest,
                    RValue::Use(Operand::Constant(Constant::Void)),
                ));
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
                    x => todo!("{x:?} not yet implemented"),
                }
            }

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



    fn lower_pattern(&mut self, pattern: PatternId, val_local: LocalId) {
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


        match self.ast.get_pattern(pattern) {
            Pattern::Binding { .. } => {
                let var_id = self.typed_ast.resolved_pattern_var[&pattern];
                let local = self.get_or_create_local(var_id);

                self.add_curr_statement(Statement::Assign(local, RValue::Use(Operand::Copy(val_local))));
            }

            Pattern::Wildcard => {
                // Wildcard go brrrr
            }

            Pattern::Tuple(elems) => {
                let val_local_type = self.function.locals[val_local.0 as usize].type_id;
                let mut offset = 0;

                for (i, elem) in elems.iter().enumerate() {
                    let elem_type = self.type_arena.tuple_field_type(val_local_type, i);
                    let elem_size = self.type_arena.type_size(elem_type);
                    let elem_local = self.new_temp_of_type(elem_type);

                    self.add_curr_statement(Statement::Assign(
                        elem_local,
                        RValue::TupleGet { tuple: val_local, offset },
                    ));
                    offset += elem_size;

                    self.lower_pattern(elem.pattern, elem_local);
                }
            }

            Pattern::PlacePointer(expr) => {
                let ptr_local = self.new_temp_expr(*expr);
                self.add_curr_statement(Statement::Store {
                    pointer: ptr_local,
                    value: Operand::Copy(val_local),
                });
            }

            x => todo!("{x:?} not yet implemented...")
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