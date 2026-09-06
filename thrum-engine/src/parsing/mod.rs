use crate::{ErrType, ProgramError, ProgramErrorData, WarnType, lexing::{self, tokens::{AssignOp, Span, TokenKind, TokenSpan}}, parsing::{ast::{AstArena, Expr, ExprId, Pattern, PatternId}, parse_expressions::ParserCtx}};

pub mod ast;
pub mod desugar;
mod parse_expressions;
mod parse_patterns;



pub struct Parser<'a> {
    error_data: &'a mut ProgramErrorData,
    source: &'a str,
    tokens: &'a [TokenSpan],
    curr_token_pos: usize,
    ast: &'a mut AstArena,
    prev_token_span: Span,
}

impl<'a> Parser<'a> {
    pub fn start(error_data: &mut ProgramErrorData, source: &str, tokens: &[TokenSpan]) -> AstArena {
        let mut ast = AstArena::default();
        // reserve the first slot of the ast
        ast.exprs.push(Expr::ParserError);
        ast.expr_spans.push(Span::invalid());

        Parser {
            error_data, source, tokens,
            curr_token_pos: 0,
            ast: &mut ast,
            prev_token_span: Span::default()
        }
        .parse_block_expression(TokenKind::EndOfFile, Span::default());

        // swap the last expression (which is the main block expression)
        // into the reserved first slot.
        ast.exprs.swap_remove(0);
        ast.expr_spans.swap_remove(0);

        ast
    }

    fn peek(&self) -> &'a TokenSpan {
        self.tokens.get(self.curr_token_pos).unwrap_or(&TokenSpan::END_TOKEN)
    }
    fn peek_one_further(&self) -> &'a TokenSpan {
        // needed for tuple label parsing (x: 2, y: 3) vs (2, 3)
        // we never peek further than this though, the language is designed to be easy to parse
        self.tokens.get(self.curr_token_pos + 1).unwrap_or(&TokenSpan::END_TOKEN)
    }

    fn next(&mut self) -> &'a TokenSpan {
        let next = self.tokens.get(self.curr_token_pos).unwrap_or(&TokenSpan::END_TOKEN);
        self.curr_token_pos += 1;
        self.prev_token_span = next.span;
        next
    }

    fn peek_is_on_same_line(&self) -> bool {
        self.peek().span.line == self.prev_token_span.line
    }
    fn peek_further_is_on_same_line(&self) -> bool {
        self.peek_one_further().span.line == self.prev_token_span.line
    }

    fn peek_spaces_before(&self) -> usize {
        self.peek().span.byte_offset - (self.prev_token_span.byte_offset + self.prev_token_span.length)
    }
    fn peek_spaces_after(&self) -> usize {
        self.peek_one_further().span.byte_offset - (self.peek().span.byte_offset + self.peek().span.length)
    }

    #[track_caller]
    fn error(&mut self, err_type: ErrType) {
        let span = self.peek().span;
        self.error_with_span(err_type, span);
    }
    #[track_caller]
    fn error_with_span(&mut self, err_type: ErrType, span: Span) {
        self.error_data.errors.push(ProgramError {
            span,
            err_type,
            compiler_location: std::panic::Location::caller()
        });
    }
    #[track_caller]
    fn warn(&mut self, warn_type: WarnType) {
        let span = self.peek().span;
        self.warn_with_span(warn_type, span);
    }
    #[track_caller]
    fn warn_with_span(&mut self, warn_type: WarnType, span: Span) {
        self.error_data.warnings.push(ProgramError {
            span,
            err_type: warn_type,
            compiler_location: std::panic::Location::caller()
        });
    }

    fn add_expr(&mut self, start: Span, expr: Expr) -> ExprId {
        self.ast.add_expr(start.merge(self.prev_token_span), expr)
    }
    fn add_pattern(&mut self, start: Span, pattern: Pattern) -> PatternId {
        self.ast.add_pattern(start.merge(self.prev_token_span), pattern)
    }

    fn expect_token(&mut self, expected: TokenKind, error_msg: &str) {
        let peek = self.peek().clone();
        if peek.token == expected {
            self.next();
        } else {
            self.error(ErrType::ParserExpectToken { expected: [expected].into(), err_msg: error_msg.to_string(), found: peek.token });
        }
    }
    fn expect_identifier(&mut self, err_msg: &str) -> String {
        let peek = self.peek();
        if peek.token == TokenKind::Identifier {
            self.next();
            self.get_from_source(peek.span).to_string()
        } else {
            self.error(ErrType::ParserExpectToken { expected: [TokenKind::Identifier].into(), err_msg: err_msg.to_string(), found: peek.token });
            String::new()
        }
    }

    fn expect_identifier_relaxed(&mut self, err_msg: &str) -> String {
        // can be a normal identifier, keyword or number
        let peek = self.peek().clone();
        if peek.token == TokenKind::NumInt || TokenKind::KEYWORDS.iter().any(|(_, k)| *k == peek.token) {
            self.next();
            self.get_from_source(peek.span).to_string()
        } else {
            self.expect_identifier(err_msg)
        }
    }

    fn optional_token(&mut self, expected: TokenKind) -> bool {
        let matched = self.peek().token == expected;
        if matched {
            self.next();
        }
        matched
    }

    fn optional_label(&mut self) -> Option<String> {
        let label_on_same_line = self.prev_token_span.line == self.peek().span.line;
        self.optional_token(TokenKind::Hashtag)
            .then(|| {
                if !label_on_same_line {
                    self.error(ErrType::ParserLabelsHaveToBeOnSameLine);
                }
                self.expect_identifier_relaxed("to name the label")
            })
    }

    fn optional_string_frag(&mut self) -> String {
        if self.optional_token(TokenKind::StringFrag) {
            let source_frag = self.get_from_source(self.prev_token_span);
            lexing::lex_string_from(source_frag)
        } else {
            String::new()
        }
    }

    fn get_from_source(&self, span: Span) -> &str {
        &self.source[span.byte_offset..(span.byte_offset + span.length)]
    }

    fn parse_comma_separated<T>(
        &mut self,
        end_token: TokenKind,
        parse_element: impl Fn(&mut Self, i32) -> T,
        err_msg: &str
    ) -> Vec<T>
    {
        let mut list = Vec::new();

        // handles empty lists immediately
        for i in 0.. {
            if self.peek().token == end_token { break }
            list.push(parse_element(self, i));
            if !self.optional_token(TokenKind::Comma) { break; }
        }
        self.expect_token(end_token, err_msg);
        list
    }

    fn parse_line_seperated(&mut self, end_token: TokenKind, consume_end: bool, parse_ctx: ParserCtx, add_semicolon_expr: bool) -> Vec<ExprId> {
        let mut list = Vec::new();

        while self.peek().token != end_token && self.peek().token != TokenKind::EndOfFile {
            list.push(self.parse_expression_default(parse_ctx));
            if self.optional_token(TokenKind::Semicolon) {
                if add_semicolon_expr {
                    list.push(self.add_expr(self.prev_token_span, Expr::Void));
                }
            }
            else if self.peek_is_on_same_line() && self.peek_is_expression_start() {
                // no semicolon -> next expression can't be on the same line.
                // (if its actually an expression and not just '}')
                self.error(ErrType::MultipleExprsWithoutSemicolon);
            }
        }
        if consume_end && end_token != TokenKind::EndOfFile {
            self.expect_token(end_token, "to close the block");
        }
        list
    }

    fn peek_is_expression_start(&self) -> bool {
        // should match self.parse_prefix()
        matches!(self.peek().token,
            TokenKind::Exclamation | TokenKind::Op(AssignOp::Minus | AssignOp::Star) | TokenKind::Identifier
            | TokenKind::Mut | TokenKind::NumInt | TokenKind::NumFloat | TokenKind::Bool(_)
            | TokenKind::LeftBrace | TokenKind::LeftParen | TokenKind::StringStart | TokenKind::Let | TokenKind::Const | TokenKind::Type
            | TokenKind::If | TokenKind::Ensure | TokenKind::While | TokenKind::For | TokenKind::Loop | TokenKind::Match | TokenKind::Enum
            | TokenKind::Impl | TokenKind::ImplSelf | TokenKind::Colon | TokenKind::Fn | TokenKind::Pipe
            | TokenKind::Return | TokenKind::Break | TokenKind::Continue | TokenKind::Ampersand
        )
    }

    // fn recover(&mut self, recover_after_tokens: &[TokenKind], recover_on_tokens: &[TokenKind]) {
    //     while self.peek().token != TokenKind::EndOfFile {
    //         if recover_on_tokens.contains(&self.peek().token)
    //         || recover_after_tokens.contains(&self.next().token) {
    //             break
    //         }
    //     }
    // }
}