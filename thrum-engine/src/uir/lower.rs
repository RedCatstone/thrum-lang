use std::collections::HashMap;

use crate::{
    parsing::ast::{AstArena, Expr, ExprId, Pattern, PatternId}, typing::{TypeId, TypeVarId, TypedAst}, uir::ir::{
        BlockId, Constant, FunctionId, LocalId, Operand, RValue, Statement, Terminator, UirFunction,
    },
};

pub struct FnLowerer<'a> {
    ast: &'a AstArena,
    typed_ast: &'a TypedAst,
    function: UirFunction,
    current_block: BlockId,
    var_to_local: HashMap<TypeVarId, LocalId>,
}

impl<'a> FnLowerer<'a> {
    #[must_use] pub fn start(ast: &'a AstArena, typed_ast: &'a TypedAst) -> UirFunction {
        let mut lowerer = Self::new(ast, typed_ast);
        lowerer.lower_function_root(ExprId(0));
        lowerer.function
    }

    fn new(ast: &'a AstArena, typed_ast: &'a TypedAst) -> Self {
        let return_type = typed_ast.get_expr_type(ExprId(0));
        let function = UirFunction::new(FunctionId(0), "main".to_string(), 0, return_type);
        let current_block = function.entry_block;

        Self { ast, typed_ast, function, current_block, var_to_local: HashMap::new() }
    }

    fn lower_function_root(&mut self, root: ExprId) {
        self.lower_expr_to_local(root, LocalId::RETURN_PLACE);
        self.function.set_terminator(self.current_block, Terminator::Return);
    }

    /// lowers the expression so it directly ends up in `dest`.
    pub fn lower_expr_to_local(&mut self, expr_id: ExprId, dest: LocalId) {
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
                for expr in exprs {
                    self.lower_expr_to_local(*expr, dest);
                }
            }

            Expr::Assign { pattern, value, extra_op, op_span: _ } => {
                let val_place = self.new_temp_expr(*value);
                self.lower_pattern(*pattern, val_place);
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

            other => todo!("support for {other:?} not yet implemented"),
        }

        // auto-derefs from the typechecker
        if let Some(&derefs) = self.typed_ast.auto_derefs.get(&expr_id) {
            for _ in 0..derefs {
                self.add_curr_statement(Statement::Assign(dest, RValue::Deref(dest)));
            }
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
        let temp = self.function.new_temp(typ);

        self.lower_expr_to_local(expr, temp);
        temp
    }

    fn add_curr_statement(&mut self, stmt: Statement) {
        self.function.add_statement(self.current_block, stmt);
    }



    fn lower_pattern(&mut self, pattern: PatternId, val_local: LocalId) {
        let pointer_count = self.typed_ast.resolved_pattern_derefs.get(&pattern).copied().unwrap_or(0);
        for _ in 0..pointer_count {
            self.add_curr_statement(Statement::Assign(val_local, RValue::Deref(val_local)));
        }


        match self.ast.get_pattern(pattern) {
            Pattern::Binding { .. } => {
                let var_id = self.typed_ast.resolved_pattern_var[&pattern];

                let local = match self.var_to_local.get(&var_id) {
                    Some(&l) => l,
                    None => {
                        let var = self.typed_ast.get_var(var_id);
                        let l = self.function.new_local(var.typ, Some(var.name.clone().into_boxed_str()), var.is_declared_mut);
                        self.var_to_local.insert(var_id, l);
                        l
                    }
                };

                self.add_curr_statement(Statement::Assign(local, RValue::Use(Operand::Copy(val_local))));
            }

            x => todo!("{x:?} not yet implemented...")
        }
    }
}