use crate::{
    ErrType, WarnType, lexing::{self, tokens::{AssignOp, Span, TokenKind, TokenSpan}},
    parsing::{Parser, ast::{AstClosure, AstEnumExpression, AstMatchArm, AstTupleElement, AstValue, Expr, ExprId}}
};



#[derive(Debug, PartialEq, PartialOrd, Clone, Copy)]
pub enum Precedence {
    Lowest,
    Assign,     // =, +=, -=, etc.
    Range,      // 1..2
    // Nullish,    // ??
    Or,         // |
    And,        // &
    Comparison, // ==, !=
    LessGreater,// <, >, <=, >=
    Is,         // is
    Sum,        // +, -
    Product,    // *, /, %
    Prefix,     // ! -
    CallIndex,  // square(X), array[i], arr.len, Option::Some
    Postfix,    // ^
}
impl Precedence {
    pub const fn get_precedence(token_type: TokenKind) -> Self {
        match token_type {
            TokenKind::Assign { .. } => Self::Assign,
            TokenKind::DotDot | TokenKind::DotDotEqual => Self::Range,
            TokenKind::Or => Self::Or,
            TokenKind::And => Self::And,
            TokenKind::EqualEqual | TokenKind::NotEqual => Self::Comparison,
            TokenKind::Less | TokenKind::Greater | TokenKind::LessEqual | TokenKind::GreaterEqual => Self::LessGreater,
            TokenKind::Is => Self::Is,
            TokenKind::Op(AssignOp::Plus | AssignOp::Minus) => Self::Sum,
            TokenKind::Op(AssignOp::Star | AssignOp::Slash | AssignOp::Percent) => Self::Product,
            TokenKind::LeftParen | TokenKind::LeftBracket | TokenKind::Dot | TokenKind::ColonColon => Self::CallIndex,
            TokenKind::Caret => Self::Postfix,
            _ => Self::Lowest,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ParserCtx {
    /// toggled on in match arms
    /// toggled back off in {}, (), []
    pub stop_on_newline_is: bool,
}

#[derive(PartialEq)]
/// The language only has a parser distinction between statements and expressions.
/// Just because it dissallows more bogus code like `let x = let y = 3` and so it stops
/// parsing infix operators after e.g. `impl {}` or `fn ...`.
pub enum StmtKind {
    Stmt, Expr, Let, NestedLet
}


impl Parser<'_> {
    pub(super) fn parse_expression_default(&mut self, ctx: ParserCtx) -> ExprId {
        self.parse_expression(Precedence::Lowest, ctx)
    }

    pub(super) fn parse_expression(&mut self, precedence: Precedence, ctx: ParserCtx) -> ExprId {
        self.parse_expression_or_statement(precedence, ctx, false).0
    }

    pub(super) fn parse_expression_or_statement(&mut self, precedence: Precedence, ctx: ParserCtx, allow_stmt: bool) -> (ExprId, StmtKind) {
        // examples of prefixes:
        // 1
        // !(1 + 2)
        // Vec::new(data = [1, 2, 3])
        // if x { 1 } else { 2 }
        let (mut left_expr, mut is_stmt) = self.parse_prefix(ctx);

        // Pratt parser loop
        loop {
            let peek_op = self.peek().clone();

            match is_stmt {
                StmtKind::Stmt => break, // statements (x = 2, fn ...) can't consume infix operators
                StmtKind::Let | StmtKind::NestedLet
                // `let` or `(let a, b)` can ONLY be followed by `=`
                if peek_op.token != (TokenKind::Assign { extra_op: None }) => break,
                _ => {}
            }

            let op_precedence = Precedence::get_precedence(peek_op.token);

            // Not an infix operator.
            if op_precedence == Precedence::Lowest { break }

            // only includes the next operator if it binds stronger than the current one.
            // 1 + 2 * 3   -> this would consume until * and only afterwards process +
            // 1 * 2 + 3   -> this would stop at * and process + afterwards
            if precedence >= op_precedence { break }

            // 1 level lower for right associativity
            // ** is special because 2**3**2 should get parsed as: 2**(3**2)
            // if op_token.token == TokenKind::StarStar { op_precedence = Precedence::Product }

            // operators that are not allowed to be line-split.
            // this is so semicolons are never actually needed
            // for example here the parser would normally want to keep consuming the ( as a function call
            // let mut a = 1
            // (a, b) = (b, a + b)
            if !self.peek_is_on_same_line() && (
                matches!(peek_op.token, TokenKind::LeftParen | TokenKind::LeftBracket)
                || (matches!(peek_op.token, TokenKind::Is) && ctx.stop_on_newline_is)
            ) {
                break;
            }

            // special handling for normal Ops: + - * / ...
            if let TokenKind::Op(_) = peek_op.token {
                // this is an infix operator: `5 \n - 2`
                // NOT an infix operator:     `5 \n -2`
                if !self.peek_is_on_same_line() && self.peek_spaces_after() == 0 {
                    break;
                }

                if self.peek_is_on_same_line() && self.peek_further_is_on_same_line() {
                    // warnings for uneven spacing around infix operator `5- 2`
                    if self.peek_spaces_before() != self.peek_spaces_after() {
                        self.warn(WarnType::ParserInconsistentSpacingAroundInfixOp { op: peek_op.token });
                    }
                }
            }

            self.next(); // consume the operator

            // update the left expression with the new infix result.
            (left_expr, is_stmt) = self.parse_infix(left_expr, &peek_op, op_precedence, ctx);
        }


        if (!allow_stmt && is_stmt != StmtKind::Expr) || is_stmt == StmtKind::NestedLet {
            // not allowed in expression context (e.g. `some_func((let a, let b))`, `1 + let x`)
            // or plain `(let a, let b)` without `=`
            self.error_with_span(ErrType::ParserOnlyAllowedInStatementPosition, self.ast.get_expr_span(left_expr));
        }

        (left_expr, is_stmt)
    }



    fn parse_infix(&mut self, left_expr: ExprId, op: &TokenSpan, op_precedence: Precedence, ctx: ParserCtx) -> (ExprId, StmtKind) {
        let start = self.ast.get_expr_span(left_expr);
        let mut is_stmt = StmtKind::Expr;

        if let Precedence::And | Precedence::Or | Precedence::Comparison
            | Precedence::LessGreater | Precedence::Sum | Precedence::Product = op_precedence {
            let right = self.parse_expression(op_precedence, ctx);
            let expr = self.add_expr(start, Expr::Infix { op: op.token, op_span: op.span, left: left_expr, right });
            return (expr, is_stmt)
        }

        let expr = match op.token {
            TokenKind::Dot => {
                // supports both x.member and x.2
                let member = self.expect_identifier_relaxed("to name the member");
                let member_expr = self.add_expr(start, Expr::MemberAccess { left: left_expr, member });
                self.wrap_in_optional_type_instantiation(member_expr)
            },

            TokenKind::ColonColon => {
                let member = self.expect_identifier("to name the path");
                self.add_expr(start, Expr::TypeMemberAccess { left: left_expr, member })
            }

            TokenKind::Assign { extra_op } => {
                is_stmt = StmtKind::Stmt;
                let pattern = self.convert_expr_into_assign_pattern(left_expr);
                let value = self.parse_expression_default(ctx);

                self.add_expr(start, Expr::Assign { pattern, value, extra_op, op_span: op.span })
            },

            TokenKind::Caret => {
                // move/clone operator
                self.add_expr(start, Expr::Move { expr: left_expr })
            },

            TokenKind::Is => {
                let pattern = self.parse_pattern(false, ctx);

                self.add_expr(start, Expr::Is { value: left_expr, pattern })
            }

            TokenKind::LeftParen => {
                let arguments = self.parse_comma_seperated_expressions(
                    TokenKind::RightParen,
                    "to close the function arguments list"
                );
                self.add_expr(start, Expr::Call { callee: left_expr, arguments })
            },

            TokenKind::LeftBracket => {
                let index = self.parse_expression_default(ctx);
                self.expect_token(TokenKind::RightBracket, "to close the index expression");
                self.add_expr(start, Expr::Index { left: left_expr, index })
            },

            TokenKind::DotDot => {
                let right = self.parse_expression_default(ctx);

                let range_type = self.add_expr(start, Expr::IdentifierRef { name: "Range".to_string(), mutable: false });
                let data = self.add_expr(start, Expr::Tuple { elems: vec![
                    AstTupleElement { label: "start".to_string(), expr: left_expr },
                    AstTupleElement { label: "end".to_string(), expr: right },
                ] });
                self.add_expr(start, Expr::TypeInstantiation { typ: range_type, data })
            }

            _ => unreachable!("parse_infix() should not be called with op_token: {op:?}")
        };

        (expr, is_stmt)
    }









    pub(super) fn parse_prefix(&mut self, ctx: ParserCtx) -> (ExprId, StmtKind) {
        // just for making 100% sure that this function is always in sync, even if i edit stuff.
        // (it being out of sync already lead to some weird bugs before...)
        let claims_to_be_start = self.peek_is_expression_start();

        let op = self.next();
        let start = op.span;
        let mut is_stmt = StmtKind::Expr;

        let expr = match op.token {
            TokenKind::Exclamation | TokenKind::Op(AssignOp::Minus) => {
                if self.peek_spaces_before() > 0 {
                    self.error_data.warn(WarnType::ParserUnnecessarySpacingAfterPrefixOp { op: op.token }, op.span);
                }

                let right = self.parse_expression(Precedence::Prefix, ctx);
                self.add_expr(start, Expr::Prefix { op: op.token, right })
            }

            TokenKind::Op(AssignOp::Star) => {
                let expr = self.parse_expression(Precedence::Prefix, ctx);
                self.add_expr(start, Expr::Move { expr })
            }

            TokenKind::Identifier => {
                let name = self.get_from_source(op.span).to_string();
                let name_expr = self.add_expr(start, Expr::IdentifierRef { name, mutable: false });
                self.wrap_in_optional_type_instantiation(name_expr)
            }

            TokenKind::Mut => {
                let name = self.expect_identifier("after mut");
                self.add_expr(start, Expr::IdentifierRef { name, mutable: true })
            }

            TokenKind::NumInt => {
                let num = lexing::lex_num_int_from(self.get_from_source(start));
                self.add_expr(start, Expr::Literal { val: AstValue::NumInt(num) })
            }
            TokenKind::NumFloat => {
                let num = lexing::lex_num_float_from(self.get_from_source(start));
                self.add_expr(start, Expr::Literal { val: AstValue::NumFloat(num) })
            }

            TokenKind::Bool(val) => self.add_expr(start, Expr::Literal { val: AstValue::Bool(val) }),

            TokenKind::LeftBrace => self.parse_block_expression(TokenKind::RightBrace, start),

            TokenKind::LeftParen => {
                let (expr, tup_stmt) = self.parse_tuple_or_arr_expression(start, TokenKind::RightParen, true, true);
                is_stmt = tup_stmt;
                expr
            }

            TokenKind::StringStart => {
                let mut elems = Vec::new();
                let mut only_string_frags = true;

                while !self.optional_token(TokenKind::StringEnd) {
                    if self.optional_token(TokenKind::StringFrag) {
                        // extract the string from the source
                        // we also need to handle backslashes here
                        let source_frag = self.get_from_source(self.prev_token_span);
                        let s = lexing::lex_string_from(source_frag);

                        elems.push(self.add_expr(self.prev_token_span, Expr::Literal { val: AstValue::Str(s) }));
                    }
                    if self.optional_token(TokenKind::LeftBrace) {
                        elems.push(self.parse_expression_default(ctx));
                        only_string_frags = false;
                        self.expect_token(TokenKind::RightBrace, "to close string interpolation");
                    }
                }

                match elems[..] {
                    [first] if only_string_frags => first,
                    [] => self.add_expr(start, Expr::Literal { val: AstValue::Str(String::new()) }),
                    _ => self.add_expr(start, Expr::TemplateString { elems }),
                }
            },

            TokenKind::Let => {
                is_stmt = StmtKind::Let;
                // this only adds an EmptyLet expr, because let has multiple use cases
                // e.g.: `(a, let b) = 2`  `x is let .Some(a)`
                let pattern = self.parse_pattern(true, ctx);
                self.add_expr(start, Expr::EmptyLet { pattern })
            },

            TokenKind::Const => {
                is_stmt = StmtKind::Stmt;
                let pattern = self.parse_pattern(true, ctx);
                self.expect_token(TokenKind::Assign { extra_op: None }, "to assign a value to the const.");
                let value = self.parse_expression_default(ctx);

                self.add_expr(start, Expr::Const { pattern, value })
            }
            TokenKind::Type => {
                is_stmt = StmtKind::Stmt;
                let name = self.expect_identifier("to name the type").into_boxed_str();
                self.expect_token(TokenKind::Assign { extra_op: None }, "to assign a value to the type.");
                let value = self.parse_expression_default(ctx);

                self.add_expr(start, Expr::CustomType { name, value })
            }

            TokenKind::If => {
                let condition = self.parse_expression(Precedence::Lowest, ctx);
                let then = self.parse_arrow_or_block_expression("if", ctx);
                let alt = if self.optional_token(TokenKind::Else) {
                        self.parse_expression_default(ctx)
                    } else {
                        let then_span = self.ast.get_expr_span(then);
                        self.add_expr(then_span.to_0_width_right(), Expr::Void)
                    };
                self.add_expr(start, Expr::If { condition, then, alt, never_alt: false })
            },

            TokenKind::Ensure => {
                is_stmt = StmtKind::Stmt;
                let condition = self.parse_expression(Precedence::Lowest, ctx);
                self.expect_token(TokenKind::Else, "after the ensure condition");
                let alt = self.parse_expression_default(ctx);

                if !self.optional_token(TokenKind::Semicolon)
                && self.peek_is_on_same_line() && self.peek_is_expression_start() {
                    self.error(ErrType::MultipleExprsWithoutSemicolon);
                }

                let then_start = self.prev_token_span.to_0_width_right();
                let then_exprs = self.parse_line_seperated(TokenKind::RightBrace, false, ctx, true);
                let then = self.add_expr(then_start, Expr::Block { exprs: then_exprs, label: None });

                self.add_expr(start, Expr::If { condition, then, alt, never_alt: true })
            },

            TokenKind::While => {
                let label = self.optional_label().unwrap_or_else(|| "while".to_string());
                let condition = self.parse_expression_default(ctx);
                let body = self.parse_arrow_or_block_expression("while", ctx);

                self.add_expr(start, Expr::While { condition, body, label })
            },

            TokenKind::For => {
                let label = self.optional_label().unwrap_or_else(|| "for".to_string());
                let pattern = self.parse_pattern(true, ctx);
                self.expect_token(TokenKind::In, "after for-loop pattern");
                let iter_expr = self.parse_expression_default(ctx);
                let body = self.parse_arrow_or_block_expression("for", ctx);

                self.add_expr(start, Expr::For { pattern, iter_expr, body, label })
            },

            TokenKind::Loop => {
                let label = self.optional_label().unwrap_or_else(|| "loop".to_string());
                let body = self.parse_arrow_or_block_expression("loop", ctx);

                self.add_expr(start, Expr::Loop { body, label })
            },

            TokenKind::Match => {
                let match_value = self.parse_expression(Precedence::Is, ctx);
                let mut arms = Vec::new();

                while self.optional_token(TokenKind::Is) {
                    let pattern = self.parse_pattern(false, ctx);
                    let body = self.parse_arrow_or_block_expression("match arm", ParserCtx { stop_on_newline_is: true });
                    self.optional_token(TokenKind::Comma);
                    arms.push(AstMatchArm { pattern, body });
                }

                self.add_expr(start, Expr::Match { match_value, arms })
            },

            TokenKind::Enum => {
                // enum { Some(T), None }
                self.expect_token(TokenKind::LeftBrace, "to open the enum definition block");
                let variants = self.parse_comma_separated(
                    TokenKind::RightBrace,
                    |p, _| p.parse_enum_variant(),
                    "to close the enum definition block"
                );
                self.add_expr(start, Expr::EnumDefinition { variants })
            },

            TokenKind::Impl => {
                is_stmt = StmtKind::Stmt;
                let typ = self.parse_expression_default(ctx);
                self.expect_token(TokenKind::LeftBrace, "to open the impl definition block");

                let const_exprs = self.parse_line_seperated(TokenKind::RightBrace, true, ctx, false);

                self.add_expr(start, Expr::ImplBlock { typ, const_exprs })
            }

            TokenKind::ImplSelf => {
                self.add_expr(start, Expr::ImplSelf)
            }

            TokenKind::Colon => {
                // enum variant!
                let data = self.parse_enum_variant();
                self.add_expr(start, Expr::EnumVariant { data })
            }

            TokenKind::Fn => {
                is_stmt = StmtKind::Stmt;
                let name = self.expect_identifier("to name the function").into_boxed_str();
                self.expect_token(TokenKind::LeftParen, "to open the fn definition paramter list");

                let params = self.parse_comma_separated(
                    TokenKind::RightParen,
                    |p, _| p.parse_pattern(true, ctx),
                    "to close the fn definition parameter list"
                )
                .into_boxed_slice();

                let return_type = self.optional_token(TokenKind::MinusArrow)
                    .then(|| self.parse_expression(Precedence::Lowest, ctx));

                let body = self.parse_arrow_or_block_expression("function body", ctx);

                self.add_expr(start, Expr::FnDefinition { name, closure: AstClosure { params, return_type, body } })
            },

            TokenKind::Pipe => {
                // Closure!
                let params = self.parse_comma_separated(
                    TokenKind::EqualArrow,
                    |p, _| p.parse_pattern(true, ctx),
                    "to close the fn definition parameter list"
                )
                .into_boxed_slice();

                let body = self.parse_expression_default(ctx);

                self.add_expr(start, Expr::Closure { closure: AstClosure { params, return_type: None, body }, requires_type_annotation: false })
            }

            TokenKind::Return => {
                let expr = self.parse_optional_expression_or_void(ctx);
                self.add_expr(start, Expr::Return { expr })
            },

            TokenKind::Break => {
                let label = self.optional_label();
                let expr = self.parse_optional_expression_or_void(ctx);
                self.add_expr(start, Expr::Break { expr, label })
            },

            TokenKind::Continue => {
                let label = self.optional_label();
                self.add_expr(start, Expr::Continue { label })
            }

            TokenKind::Ampersand => {
                let mutable = self.optional_token(TokenKind::Mut);
                let expr = self.parse_expression(Precedence::Prefix, ctx);
                self.add_expr(start, Expr::Borrow { expr, mutable })
            }

            _ => {
                debug_assert!(!claims_to_be_start, "SYNC BUG: {:?} is not in `parse_prefix()`", op.token);

                self.error_with_span(ErrType::ParserExpectedAnExpression { found: op.token }, op.span);
                return (self.add_expr(start, Expr::ParserError), is_stmt)
            }
        };

        debug_assert!(claims_to_be_start, "SYNC BUG: {:?} is not in `peek_is_expression_start()`", op.token);

        (expr, is_stmt)
    }





    pub(super) fn parse_block_expression(&mut self, end_token: TokenKind, start: Span) -> ExprId {
        // '{' already consumed.
        let label = self.optional_label();

        let exprs = self.parse_line_seperated(end_token, true, ParserCtx { stop_on_newline_is: false }, true);

        self.add_expr(start, Expr::Block { exprs, label })
    }

    pub(super) fn parse_arrow_or_block_expression(&mut self, block_name: &str, ctx: ParserCtx) -> ExprId {
        if self.optional_token(TokenKind::EqualArrow) {
            // => ...
            if !self.peek_is_on_same_line() {
                self.error(ErrType::ParserArrowExprsHaveToBeOnSameLine);
            }
            let expr = self.parse_expression_default(ctx);
            self.add_expr(self.prev_token_span, Expr::Block { exprs: vec![expr], label: None })
        }
        else if self.optional_token(TokenKind::LeftBrace) {
            // { ... }
            self.parse_block_expression(TokenKind::RightBrace, self.prev_token_span)
        }
        else {
            self.error(ErrType::ParserExpectToken {
                expected: [TokenKind::EqualArrow, TokenKind::LeftBrace].into(),
                err_msg: format!("to open the {block_name} block"),
                found: self.peek().token
            });
            self.add_expr(self.prev_token_span, Expr::ParserError)
        }
    }


    fn parse_enum_variant(&mut self) -> AstEnumExpression {
        // Some(T)
        AstEnumExpression {
            variant_name: self.expect_identifier("to name an enum variant").into_boxed_str(),
            attached_tuple: self.optional_token(TokenKind::LeftBrace).then(|| {
                self.parse_tuple_or_arr_expression(self.prev_token_span, TokenKind::RightBrace, false, false).0
            })
        }
    }

    fn wrap_in_optional_type_instantiation(&mut self, wrap: ExprId) -> ExprId {
        // Number{ 3 } is only allowed if there is no space after 'Number'
        if self.peek_spaces_before() == 0 && self.optional_token(TokenKind::LeftBrace) {
            let data = self.parse_tuple_or_arr_expression(self.prev_token_span, TokenKind::RightBrace, false, false).0;

            let wrap_span = self.ast.get_expr_span(wrap);
            self.add_expr(wrap_span, Expr::TypeInstantiation { typ: wrap, data })
        } else {
            wrap
        }
    }





    fn parse_comma_seperated_expressions(&mut self, end_token: TokenKind, err_msg: &str) -> Vec<ExprId> {
        // '[1, 2, 3]',   '(1, 2)',   dict { 1, 2 }
        self.parse_comma_separated(
            end_token,
            |p, _| p.parse_expression_default(ParserCtx { stop_on_newline_is: false }),
            err_msg
        )
    }

    pub(super) fn parse_tuple_item<Id>(
        &mut self,
        default_label: String,
        mut parse_item: impl FnMut(&mut Self) -> Id,
        make_shorthand: impl Fn(&mut Self, String) -> Id,
    ) -> (String, Id) {
        if self.peek_one_further().token == TokenKind::Colon {
            // its labeled!
            let label = self.expect_identifier("to label the tuple element");
            self.expect_token(TokenKind::Colon, "unreachable");

            if let TokenKind::Comma | TokenKind::RightParen | TokenKind::RightBrace = self.peek().token {
                // shorthand (x:, y:)
                (label.clone(), make_shorthand(self, label))
            } else {
                // (x: 0, y: 1)
                (label, parse_item(self))
            }
        } else {
            // unlabeled tuple (1, 2)
            (default_label, parse_item(self))
        }
    }

    fn parse_one_tuple_expression(&mut self, default_label: String, allow_let: bool, is_stmt: &mut StmtKind) -> AstTupleElement {
        let mut elem_stmt = StmtKind::Expr;

        let (label, expr) = self.parse_tuple_item(
            default_label,
            |p| {
                let (expr, stmt) = p.parse_expression_or_statement(Precedence::Lowest, ParserCtx { stop_on_newline_is: false }, true);
                elem_stmt = stmt;
                expr
            },
            |p, name| p.add_expr(p.prev_token_span, Expr::IdentifierRef { name, mutable: false }),
        );

        match elem_stmt {
            StmtKind::Let | StmtKind::NestedLet if allow_let => {
                *is_stmt = StmtKind::NestedLet;
            }
            StmtKind::Expr => {}
            _ => self.error_with_span(ErrType::ParserOnlyAllowedInStatementPosition, self.ast.get_expr_span(expr))
        }

        AstTupleElement { label, expr }
    }

    fn parse_tuple_or_arr_expression(&mut self, start: Span, end_token: TokenKind, maybe_just_grouped_expr: bool, allow_let: bool) -> (ExprId, StmtKind) {
        let mut is_stmt = StmtKind::Expr;

        if self.optional_token(end_token) {
            // tuple is empty
            return (self.add_expr(start, Expr::Tuple { elems: Vec::new() }), is_stmt);
        }

        let first_elem = self.parse_one_tuple_expression("0".to_string(), allow_let, &mut is_stmt);

        let expr = if self.optional_token(TokenKind::Semicolon) {
            // `(true; 3)`
            let length = self.parse_expression_default(ParserCtx { stop_on_newline_is: false });
            self.expect_token(end_token, "to close the tuple array expression");
            self.add_expr(start, Expr::TupleArr { elem: first_elem.expr, length })
        }
        else if self.optional_token(TokenKind::Comma) {
            // comma => definitely a tuple
            let mut tuple_body = vec![first_elem];
            tuple_body.extend(self.parse_comma_separated(
                end_token,
                |p, i| p.parse_one_tuple_expression((i + 1).to_string(), allow_let, &mut is_stmt),
                "to close the tuple"
            ));
            self.add_expr(start, Expr::Tuple { elems: tuple_body })
        }
        else if maybe_just_grouped_expr {
            self.expect_token(end_token, "to close the grouped expression");
            first_elem.expr
        } else {
            // 1-element tuple
            self.expect_token(end_token, "to close the tuple");
            self.add_expr(start, Expr::Tuple { elems: vec![first_elem] })
        };

        (expr, is_stmt)
    }


    fn parse_optional_expression_or_void(&mut self, ctx: ParserCtx) -> ExprId {
        if self.peek_is_on_same_line() && self.peek_is_expression_start() {
            self.parse_expression_default(ctx)
        } else {
            self.add_expr(self.prev_token_span.to_0_width_right(), Expr::Void)
        }
    }
}