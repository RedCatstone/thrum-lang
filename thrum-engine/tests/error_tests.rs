use thrum_engine::{ErrType, lexing::tokens::{AssignOp, TokenKind}};
mod common;



#[test] fn lexer_errors() {
    test_err!("1 + ~", ErrType::LexerUnexpectedCharacter { c: '~' });
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


#[test] fn typecheck_var_errors() {
    test_err!("x = 10", ErrType::TyperUndefinedIdentifier { .. });
    test_err!("let x = 10; x = 20", ErrType::TyperVarIsntDeclaredMut { .. });
    test_err!("let tup = (1, 2); tup[0] = 99", ErrType::TyperVarIsntDeclaredMut { .. });

    test_err!("let x: int; x + 5", ErrType::TyperCantUseUninitializedVar { .. });
    test_err!("let x; if true { x = 2 }; x^", ErrType::TyperCantUseMaybeInitializedVar { .. });

    test_err!("let s = \"hello\"; s^; s^", ErrType::TyperCantUseMovedVar { .. });
    test_err!("let s = \"blub\"; if true { s^ }; s^", ErrType::TyperCantUseMaybeMovedVar { .. });

    test_err!("if 42 is let x => 0 else x^", ErrType::TyperUndefinedIdentifier { .. });
    test_err!("ensure 42 is let x else panic(\"{x^}\")", ErrType::TyperUndefinedIdentifier { .. });
}

#[test] fn type_mismatch_simple() {
    test_err!("if true { 1 } else { (1,) }", ErrType::TyperMismatch { .. });
    test_err!("let x: int = true", ErrType::TyperMismatch { .. });
    test_err!("let x: int = 2.0", ErrType::TyperMismatch { .. });
    test_err!("let x: float = 2", ErrType::TyperMismatch { .. });
    test_err!("(1, 2) is (1, 2, 3)", ErrType::TyperMismatch { .. });
}
#[test] fn type_mismatch_operators() {
    test_err!("1 + true", ErrType::TyperMismatch { .. });
    test_err!("1 % true", ErrType::TyperMismatch { .. });
    test_err!("1 + 1.0", ErrType::TyperMismatch { .. });
    test_err!("1.0 + 1", ErrType::TyperMismatch { .. });
    test_err!("!5", ErrType::TyperMismatch { .. });
    test_err!("-true", ErrType::TyperMismatch { .. });
}
#[test] fn type_mismatch_patterns() {
    test_err!("1 is \"hello\"", ErrType::TyperMismatch { .. });
    test_err!("(1, 2) is (1, \"two\")", ErrType::TyperMismatch { .. });
    test_err!("(1, 2) is 1 | 2", ErrType::TyperMismatch { .. });

    test_err!("let mut x = 5; x += true", ErrType::TyperMismatch { .. });
}

#[test] fn type_mismatch_enums() {
    test_err!("
        fn handle_some(:Some{ inner }: Option.Some) -> int => inner
        handle_some(:None)
    ", ErrType::TyperMismatch { .. });
}

#[test] fn typecheck_misc_errors() {
    test_err!("loop { break #outer }", ErrType::TyperUndefinedLoopLabel { .. });
    test_err!("break", ErrType::TyperBreakOutsideLoop);
    test_err!("loop { fn sneaky() => break }", ErrType::TyperBreakOutsideLoop);
    test_err!("return", ErrType::TyperReturnOutsideFunction);
    test_err!("Self", ErrType::TyperSelfOutsideImplBlock);

    test_err!("let x", ErrType::TyperCantInferType { .. });
}

#[test] fn invalid_op_types() {
    test_err!("let x = 5; x()", ErrType::TyperCantCallNonFnType { .. });
    test_err!("let x = true; x[0]", ErrType::TyperCantIndexNonArrType { .. });

    test_err!("let x = 5; x^^", ErrType::TyperCantDerefNonPointerType { .. });
    test_err!("5^", ErrType::TyperCantDerefNonPointerType { .. });

    test_err!("let x = 5; x.hello", ErrType::TyperTypeDoesntHaveMember { .. });
    test_err!("true.hello", ErrType::TyperTypeDoesntHaveMember { .. });
}

#[test] fn exhaustive_pattern_matching() {
    test_err!("match 5 is 1 => 2", ErrType::TyperPatternDoesntCoverAllCases { .. });
    test_err!("
        match (true, false)
        is (true, true) => 0
        is (false, _) => 1
    ", ErrType::TyperPatternDoesntCoverAllCases { .. });
    test_err!("
        const Opt = enum { None, Some{ bool } }
        let val: Opt = :Some{ true }
        match val^ is :None | :Some{ true } => 0
    ", ErrType::TyperPatternDoesntCoverAllCases { .. });
    test_err!("
        const Opt = enum { None, Some{ bool } }
        let val: Opt.Some = :Some{ true }
        match val^ is :Some{ true } => 0
    ", ErrType::TyperPatternDoesntCoverAllCases { .. });
    // TODO: test_err!("match 5 is _ => 1 \n is 5 => 2", ErrType::TyperPatternCantBeReached);
}

#[test] fn pattern_binding() {
    test_err!("let a = (5 is let x)", ErrType::TyperInvalidBindingIsExpr);
    test_err!("if (5 is let x) is true {}", ErrType::TyperInvalidBindingIsExpr);

    test_err!("5 is !(let x)", ErrType::TyperNotPatternCantBindVars);

    test_err!("if (1, 2) is let (x, y) | (x, _) {}", ErrType::TyperOrPatternDoesntBindVars { .. });
    test_err!("if (1, 2) is let (x, _) | (x, y) {}", ErrType::TyperOrPatternBindsVarsTooMuch { .. });

    test_err!("if (1, 2) is let (x, 0) | (0, y) { }", ErrType::TyperOrPatternDoesntBindVars { .. });
    test_err!("let (x, 0) = (1, 2)", ErrType::TyperFailableAssignPattern { .. });
}

#[test] fn string_pattern_errors() {
    test_err!(r#" "abc" is "a{_}{_}c" "#, ErrType::TyperPatternStringHolesInARow);
    test_err!(r#" "abc" is "{}" "#, ErrType::ParserExpectedAPattern { .. });
}

#[test] fn custom_type() {
    test_err!("int{ 5 }", ErrType::TyperMustBeCustomtypeType { .. });
    test_err!("type X = int; X{ 1, 2 }", ErrType::TyperNewTypesExpectOneUnlabeledExpr);
    test_err!("type X = int; X{ 1; 1 }", ErrType::TyperNewTypesExpectOneUnlabeledExpr);
    test_err!("type X = int; X{}", ErrType::TyperNewTypesExpectOneUnlabeledExpr);
    test_err!("type X = 5", ErrType::TyperMismatch { .. });

    test_err!("let x: int = :Foo", ErrType::TyperExpectedTypeIsntAnEnum { .. });
    test_err!("const E = enum { A }; let x: E = :B", ErrType::TyperEnumDoesntHaveVariant { .. });
}

#[test] fn const_errs() {
    test_err!("const X = 2 * Y; const Y = 2 * X", ErrType::TyperConstResolvingCycle);
    test_err!("const (X, X) = (1, 2)", ErrType::TyperConstNameAlreadyExists { .. });

    test_err!("const mut X = 5", ErrType::TyperConstCantBeMutable);
    test_err!("type N = int; impl N { let x = 5 }", ErrType::TyperRuntimeValuesArentAllowedInImplBlocks);

    test_err!("let x = 10; const Y = x^", ErrType::TyperUndefinedIdentifier { .. });
    test_err!("let x = 10; (0; x)", ErrType::TyperExpectedConstFoundRuntimeValue { .. });
    test_err!("let x = 10; let y: x = 5", ErrType::TyperExpectedConstFoundRuntimeValue { .. });
}


#[test] fn array_bounds() {
    test_err!("let x = (1, 2, 3); x[3]^", ErrType::RuntimeError { .. });
    test_err!("let x = (1, 2, 3); x[1.0]^", ErrType::TyperMismatch { .. });
    test_err!("let x = (); x[0]^", ErrType::TyperCantIndexEmptyTuple { .. });
    test_err!("let x = (1, true); x[0]^", ErrType::TyperCantIndexHeterogenousTuple { .. });
}


#[test] fn new_types() {
    test_err!("type Number = int; Number{ 320 } is 320", ErrType::TyperMismatch { .. });
    test_err!("type Number = int; let x: Number = 5", ErrType::TyperMismatch { .. });
    test_err!("type N1 = int;  type N2 = int;  N1{ 6 } + N2{ 9 }", ErrType::TyperMismatch { .. });
}


#[test] fn typecheck_functions() {
    test_err!("fn square(x: int) {}; square(2, 3)", ErrType::TyperWrongNumberOfArguments { .. });
    test_err!("fn square(x: int) {}; square()", ErrType::TyperWrongNumberOfArguments { .. });
    test_err!("fn square(x) {}", ErrType::TyperRequiresTypeAnnotation);

    test_err!("fn foo() -> int { return \"string\" }", ErrType::TyperMismatch { .. });
    test_err!("fn foo() -> int { }", ErrType::TyperMismatch { .. });
}


#[test] fn deref_non_local_pointer() {
    test_err!("type T = (str); impl T { fn f(self: &Self) { self^^; } }", ErrType::TyperCantDerefUnknownPointerType);
}

#[test] fn decay_soft_info_on_variable_bind() {
    test_err!("
        type Option = enum { None, Some{ int } };

        let x = Option.Some{ 15 }
        let :Some{ inner } = x^
        inner^
    ", ErrType::TyperFailableAssignPattern { .. });
}