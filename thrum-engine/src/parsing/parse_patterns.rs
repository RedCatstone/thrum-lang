use crate::{ErrType, lexing::tokens::{Span, TokenKind}, parsing::{Parser, ast::{AstTupleElement, AstTuplePattern, Expr, ExprId, Pattern, PatternId}, parse_expressions::{ParserCtx, Precedence}}};


impl Parser<'_> {
    /// it enters binding mode when it sees a `let`
    /// `x is let y` => Binding y
    /// `x is y` => CompareExpr(IdentifierRef y)
    fn parse_one_pattern(&mut self, binding_mode: bool, ctx: ParserCtx) -> PatternId {
        let token = self.peek();
        let start = token.span;

        match token.token {
            TokenKind::Let => {
                // enter binding mode!
                self.next();
                self.parse_pattern(true, ctx)
            }

            TokenKind::Identifier => {
                let name: Box<str> = self.get_from_source(token.span).into();
                if name.starts_with('_') {
                    // x is _
                    // x is let _
                    self.next();
                    self.add_pattern(start, Pattern::Wildcard)
                }
                else if self.peek_spaces_after() == 0 && self.peek_one_further().token == TokenKind::LeftBrace {
                    // x is N{ ... }
                    // x is let N{ ... }
                    self.next(); // consume <ident>
                    self.next(); // consume '{'
                    let typ = self.add_expr(start, Expr::IdentifierRef { name: name.to_string(), mutable: false });
                    let data = self.parse_tuple_pattern(self.prev_token_span, binding_mode, TokenKind::RightBrace, false);

                    self.add_pattern(start, Pattern::TypeDestructor { typ, data })
                }
                else if binding_mode {
                    // x is let a
                    self.next();
                    self.add_pattern(start, Pattern::Binding { name, mutable: false })
                } else {
                    // x is y
                    let expr = self.parse_expression(Precedence::And, ctx);
                    self.add_pattern(start, Pattern::CompareExpr(expr))
                }
            }

            TokenKind::LeftParen => {
                self.next(); // consume '('
                self.parse_tuple_pattern(start, binding_mode, TokenKind::RightParen, true)
            }

            TokenKind::Colon => {
                self.next();
                // .Some(T)
                let variant_name = self.expect_identifier("to name an enum variant");

                let attached_tuple = self.optional_token(TokenKind::LeftBrace).then(|| {
                    self.parse_tuple_pattern(self.prev_token_span, binding_mode, TokenKind::RightBrace, false)
                });

                self.add_pattern(start, Pattern::EnumVariant { name: variant_name, attached_tuple })
            }

            TokenKind::Exclamation => {
                self.next(); // consume '!'
                let pat = self.parse_one_pattern(binding_mode, ctx);
                self.add_pattern(start, Pattern::Not(pat))
            }


            // literals in binding mode:
            // `x is 5 + 2` would parse as `x is (5 + 2)`
            // `x is let 5 + 2` would parse as `(x is 5) + 2`
            TokenKind::NumInt | TokenKind::NumFloat | TokenKind::Bool(_) if binding_mode => {
                let (expr, _) = self.parse_prefix(ctx);
                self.add_pattern(start, Pattern::CompareExpr(expr))
            }

            TokenKind::Mut if binding_mode => {
                // let mut x = ...
                self.next();
                let name = self.expect_identifier("after mut").into_boxed_str();
                self.add_pattern(start, Pattern::Binding { name, mutable: true })
            }

            TokenKind::StringStart => {
                self.next();

                let before = self.optional_string_frag();
                let mut hole_parts = Vec::new();

                while !self.optional_token(TokenKind::StringEnd) {
                    // "...{...}..."
                    let pattern = if self.optional_token(TokenKind::LeftBrace) {
                        let pattern = self.parse_pattern(binding_mode, ctx);
                        self.expect_token(TokenKind::RightBrace, "to close string pattern interpolation");
                        pattern
                    } else {
                        unreachable!("lexer makes sure that there has to be a LeftBrace here")
                    };

                    let after = self.optional_string_frag();
                    hole_parts.push((pattern, after));
                }

                self.add_pattern(start, Pattern::String { before, hole_parts: hole_parts.into_boxed_slice() })
            }

            _ if !binding_mode => {
                // in non-binding mode any expressions are allowed
                // `x is 5`
                if self.peek_is_expression_start() {
                    let expr = self.parse_expression(Precedence::And, ctx);
                    self.add_pattern(start, Pattern::CompareExpr(expr))
                } else {
                    self.error(ErrType::ParserExpectedAPattern { found: self.peek().token });
                    self.add_pattern(start, Pattern::Wildcard)
                }
            }

            found => {
                // in binding_mode any of the arms above should've already found something...
                self.error(ErrType::ParserExpectedABindingPattern { found });
                self.add_pattern(start, Pattern::Wildcard)
            }
        }
    }


    pub(super) fn parse_pattern(&mut self, binding_mode: bool, ctx: ParserCtx) -> PatternId {
        let mut pattern = self.parse_one_pattern(binding_mode, ctx);
        let start = self.ast.get_pattern_span(pattern);

        // or pattern!
        if self.optional_token(TokenKind::Pipe) {
            let mut patterns = vec![pattern];
            loop {
                patterns.push(self.parse_one_pattern(binding_mode, ctx));
                if !self.optional_token(TokenKind::Pipe) {
                    break;
                }
            }
            pattern = self.add_pattern(start, Pattern::Or(patterns));
        }

        // optional type annotation after pattern
        if self.optional_token(TokenKind::Colon) {
            let typ = self.parse_expression(Precedence::And, ctx);
            pattern = self.add_pattern(start, Pattern::Typed { pattern, typ });
        }

        // extra condition after pattern
        if self.optional_token(TokenKind::And) {
            let cond = self.parse_expression_default(ctx);
            pattern = self.add_pattern(start, Pattern::Conditional { pattern, cond });
        }

        pattern
    }




    fn parse_one_tuple_pattern(&mut self, binding_mode: bool, default_label: String) -> AstTuplePattern {
        let (label, pattern) = self.parse_tuple_item(
            default_label,
            |p| p.parse_pattern(binding_mode, ParserCtx { stop_on_newline_is: false }),
            |p, name| p.add_pattern(p.prev_token_span, Pattern::Binding { name: name.into_boxed_str(), mutable: false })
        );
        AstTuplePattern { label, pattern }
    }

    fn parse_tuple_pattern(&mut self, start: Span, binding_mode: bool, end_token: TokenKind, maybe_just_grouped_expr: bool) -> PatternId {
        if self.optional_token(end_token) {
            // tuple is empty
            return self.add_pattern(start, Pattern::Tuple(Vec::new()))
        }

        let first_elem = self.parse_one_tuple_pattern(binding_mode, "0".to_string());

        if self.optional_token(TokenKind::Comma) {
            // comma => definitely a tuple
            let mut tuple_body = vec![first_elem];
            tuple_body.extend(self.parse_comma_separated(
                end_token,
                |p, i| p.parse_one_tuple_pattern(binding_mode, (i + 1).to_string()),
                "to close the tuple"
            ));
            self.add_pattern(start, Pattern::Tuple(tuple_body))
        }
        else if maybe_just_grouped_expr {
            self.expect_token(end_token, "to close the grouped pattern");
            first_elem.pattern
        } else {
            // 1-element tuple
            self.expect_token(end_token, "to close the tuple");
            self.add_pattern(start, Pattern::Tuple(vec![first_elem]))
        }
    }





    pub(super) fn convert_expr_into_assign_pattern(&mut self, expr: ExprId) -> PatternId {
        let start = self.ast.get_expr_span(expr);

        // anything that can be on the lhs of =
        // x = 2
        // (a, let b) = (1, 2)
        // map.get_mut("bla").unwrap() =
        match self.ast.get_expr(expr) {
            Expr::EmptyLet { pattern } => *pattern,

            Expr::IdentifierRef { name, mutable: false } if name.starts_with('_') => {
                self.add_pattern(start, Pattern::Wildcard)
            }

            Expr::Tuple { elems } => {
                let converted_elems = elems.clone()
                    .into_iter()
                    .map(|AstTupleElement { label, expr }|
                        AstTuplePattern { label, pattern: self.convert_expr_into_assign_pattern(expr) }
                    )
                    .collect();

                self.add_pattern(start, Pattern::Tuple(converted_elems))
            }

            _ => self.add_pattern(start, Pattern::PlacePointer(expr))
        }
    }
}