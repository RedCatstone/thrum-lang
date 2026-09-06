use crate::{lexing::tokens::TokenKind, parsing::ast::{AstArena, AstTuplePattern, Expr, Pattern}};



pub fn desugar_after_parsing(ast: &mut AstArena) {
    // these macros are to get around the borrow checker
    // it doesn't allow `ast.add_expr(span, ast.add_expr(...))` :(
    macro_rules! pattern {
        ($span:expr, $pattern:expr) => {{
            let pattern = $pattern;
            ast.add_pattern($span, pattern)
        }};
    }
    macro_rules! expr {
        ($span:expr, $expr:expr) => {{
            let expr = $expr;
            ast.add_expr($span, expr)
        }};
    }

    // this loops until its fully done `exprs.len()`
    // which means that newly added desugared exprs will also get desugared
    for i in 0.. {
        if i == ast.exprs.len() { break }
        let expr = ast.exprs[i].clone();
        let span = ast.expr_spans[i];

        match expr {
            // desugar `while COND { BODY }`
            // --> `loop { if COND { BODY } else break }`
            Expr::While { condition, body, label } => {
                ast.exprs[i] = Expr::Loop {
                    label,
                    body: expr!(span, Expr::If {
                        condition,
                        then: body,
                        alt: expr!(span, Expr::Break { label: None, expr: expr!(span, Expr::Void) }),
                        never_alt: false
                    })
                }
            }

            // desugar `for PATT in ITER_EXPR { BODY }`
            // --> `{ let mut i = ITER_EXPR; while i.next() is .Some(PATT) { BODY } }`
            Expr::For { pattern, iter_expr, body, label } => {
                ast.exprs[i] = Expr::Block {
                    label: None,
                    exprs: vec![
                        expr!(span, Expr::Assign {
                            extra_op: None,
                            op_span: span,
                            pattern: pattern!(span, Pattern::Binding { name: "i".into(), mutable: true }),
                            value: expr!(span, Expr::Call {
                                callee: expr!(span, Expr::MemberAccess {
                                    left: iter_expr,
                                    member: "iter".into()
                                }),
                                arguments: Vec::new()
                            })
                        }),
                        expr!(span, Expr::While {
                            body,
                            label,
                            condition: expr!(span, Expr::Is {
                                value: expr!(span, Expr::Call {
                                    callee: expr!(span, Expr::MemberAccess {
                                        left: expr!(span, Expr::IdentifierRef { name: "i".into(), mutable: true }),
                                        member: "next".into()
                                    }),
                                    arguments: Vec::new()
                                }),
                                pattern: pattern!(span, Pattern::EnumVariant {
                                    name: "Some".into(),
                                    attached_tuple: Some(pattern!(span, Pattern::Tuple(vec![AstTuplePattern { label: "0".into(), pattern }])))
                                })
                            })
                        })
                    ]
                }
            }

            // desugar `fn x() { ... }`  -->  `const x = |-> ...`
            Expr::FnDefinition { name, closure } => {
                ast.exprs[i] = Expr::Const {
                    pattern: pattern!(span, Pattern::Binding { name, mutable: false }),
                    value: expr!(span, Expr::Closure { closure, requires_type_annotation: true })
                }
            }


            Expr::Infix { op, op_span, left, right }
            if let Some(inverted_op) = match op {
                TokenKind::NotEqual => Some(TokenKind::EqualEqual), // desugar `a != b`  -->  `!(a == b)`
                TokenKind::LessEqual => Some(TokenKind::Greater),   // desugar `a <= b`  -->  `!(a > b)`
                TokenKind::GreaterEqual => Some(TokenKind::Less),   // desugar `a >= b`  -->  `!(a < b)`
                _ => None
            } => {
                ast.exprs[i] = Expr::Prefix {
                    op: TokenKind::Exclamation,
                    right: expr!(span, Expr::Infix { op: inverted_op, op_span, left, right })
                };
            }

            // modify `(b: ..., a: ...)`  -->  `(a: ..., b: ...)`
            Expr::Tuple { mut elems } => {
                elems.sort_by(|a, b| a.label.cmp(&b.label));
                ast.exprs[i] = Expr::Tuple { elems };
            }

            _ => {}
        }
    }

    // this only loops up to the original length
    for i in 0..ast.patterns.len() {
        let pattern = ast.patterns[i].clone();

        #[allow(clippy::single_match)]
        match pattern {
            // modify `(b: ..., a: ...)`  -->  `(a: ..., b: ...)`
            Pattern::Tuple(mut elems) => {
                elems.sort_by(|a, b| a.label.cmp(&b.label));
                ast.patterns[i] = Pattern::Tuple(elems);
            }

            _ => {}
        }
    }
}