use thrum_engine::{ErrType, lexing::tokens::{AssignOp, TokenKind}, vm_compiling::VmValue};
mod common;


#[test] fn numbers() {
    test!("1_000", VmValue::Int(1000));
    test!("1____0", VmValue::Int(10));
    test!("1____", VmValue::Int(1));
    test!("1____.5__", VmValue::Float(1.5));
    test!("9999999999999999999", VmValue::Int(i64::MAX));
}

#[test] fn string_escaping() {
    test!(r#"  "\n\n"  "#, VmValue::OwnedStr("\n\n".to_string()));
    test!(r#"  "\n\t-\r \0"  "#, VmValue::OwnedStr("\n\t-\r \0".to_string()));
    test!(r#"  "\:\_"  "#, VmValue::OwnedStr(":_".to_string()));
    test!(r#"  "\
                  a"  "#, VmValue::OwnedStr("a".to_string()));
}

#[test] fn comments() {
    test!("
        15 /* this is a comment
        + 4
        */
        + /* weee * / */ 2
        // + 16
        + 3  // - 2
    ", VmValue::Int(20));

    test!("//", VmValue::Void);
    test!("/**/", VmValue::Void);
    test!("/**///", VmValue::Void);
    test_err!("/*", ErrType::LexerUnterminatedMultilineComment);
}

#[test] fn unicode_lexing() {
    test!("
        let ê = 3 // 🦀
        ê^
    ", VmValue::Int(3));
}


// =========================================================================
// Errors
// =========================================================================
#[test] fn lexer_errors() {
    test_err!("1 + ~", ErrType::LexerUnexpectedCharacter { c: '~' });
    test_err!("\"", ErrType::LexerUnterminatedString);
    test_err!("unterminated \"string", ErrType::LexerUnterminatedString);
    test_err!("'{'", ErrType::LexerMissingClosingStringBrace);
}
#[test] fn parser_errors() {
    test_err!("{", ErrType::ParserExpectToken { .. });
    test_err!("fn 1", ErrType::ParserExpectToken { .. });
    test_err!("if", ErrType::ParserExpectedAnExpression { found: TokenKind::EndOfFile });
    test_err!("1 - / 1", ErrType::ParserExpectedAnExpression { found: TokenKind::Op(AssignOp::Slash) });
    test_err!("1 + 1  15", ErrType::MultipleExprsWithoutSemicolon);
    test_err!("let = 5", ErrType::ParserExpectedABindingPattern { .. });
    test_err!("{ \n #label }", ErrType::ParserLabelsHaveToBeOnSameLine);
    test_err!("if true => \n 5", ErrType::ParserArrowExprsHaveToBeOnSameLine);
}

#[test] fn parse_statement_position() {
    test_err!("2 + fn none() { }", ErrType::ParserOnlyAllowedInStatementPosition);
    test_err!("2 + let x", ErrType::ParserOnlyAllowedInStatementPosition);

    test_err!("fn fn none() { } + 2", ErrType::ParserExpectedAnExpression { .. });
    test_err!("let x + 2", ErrType::ParserExpectedAnExpression { .. });
    test_err!("fn fn none() { } 2", ErrType::MultipleExprsWithoutSemicolon);

    test_err!("(let a)", ErrType::ParserOnlyAllowedInStatementPosition);
    test_err!("(x = 2)", ErrType::ParserOnlyAllowedInStatementPosition);
    test_err!("(let a, let b)", ErrType::ParserOnlyAllowedInStatementPosition);
    test_err!("2 + (let a, let b)", ErrType::ParserOnlyAllowedInStatementPosition);
}