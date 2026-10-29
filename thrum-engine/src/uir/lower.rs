use crate::{
    parsing::ast::{AstArena, Expr, ExprId}, typing::TypedAst, uir::ir::{
        BlockId, Constant, FunctionId, LocalId, Operand, Place, RValue, Statement,
        Terminator, UirFunction,
    },
};

pub struct FnLowerer<'a> {
    ast: &'a AstArena,
    typed_ast: &'a TypedAst,
    function: UirFunction,
    current_block: BlockId,
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

        Self { ast, typed_ast, function, current_block }
    }

    fn lower_function_root(&mut self, root: ExprId) {
        let return_place = Place::from_local(LocalId::RETURN_PLACE);
        self.lower_expr_to_place(root, return_place);
        self.function.set_terminator(self.current_block, Terminator::Return);
    }

    /// lowers the expression so it directly ends up in `dest`.
    pub fn lower_expr_to_place(&mut self, expr_id: ExprId, dest: Place) {
        match self.ast.get_expr(expr_id) {
            Expr::Literal { val } => {
                let constant = Constant::from_val(val);
                self.function.add_statement(
                    self.current_block,
                    Statement::Assign(dest, RValue::Use(Operand::Constant(constant))),
                );
            }

            Expr::Infix { op, left, right, op_span: _ } => {
                let left_op = self.lower_expr_to_operand(*left);
                let right_op = self.lower_expr_to_operand(*right);
                let num_mode = self.typed_ast.resolved_num_mode[&expr_id];

                self.function.add_statement(
                    self.current_block,
                    Statement::Assign(dest, RValue::BinaryOp(*op, num_mode, left_op, right_op)),
                );
            }

            Expr::Prefix { op, right } => {
                let right_op = self.lower_expr_to_operand(*right);
                let num_mode = self.typed_ast.resolved_num_mode[&expr_id];

                self.function.add_statement(
                    self.current_block,
                    Statement::Assign(dest, RValue::UnaryOp(*op, num_mode, right_op)),
                );
            }

            Expr::Block { exprs, label: _ } => {
                if let Some(&last) = exprs.last() {
                    self.lower_expr_to_place(last, dest);
                }
            }

            other => todo!("support for {other:?} not yet implemented"),
        }
    }


    pub fn lower_expr_to_operand(&mut self, expr_id: ExprId) -> Operand {
        if let Expr::Literal { val } = self.ast.get_expr(expr_id) {
            // literals become immidiate constants
            Operand::Constant(Constant::from_val(val))
        }
        else {
            // everything else gets a quick temporary value instead
            let typ = self.typed_ast.get_expr_type(expr_id);
            let temp = self.function.new_temp(typ);
            let place = Place::from_local(temp);

            self.lower_expr_to_place(expr_id, place.clone());
            Operand::Move(place)
        }
    }
}