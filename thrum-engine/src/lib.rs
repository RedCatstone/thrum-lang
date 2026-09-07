use std::{fmt::Display, time::Instant};

use derive_more::Display;

use crate::{lexing::tokens::{Span, TokenKind}, parsing::ast::ExprId, pretty_printing::{format_program_error, slice_to_or_string, slice_to_string}, typing::type_vars::TypeVar, vm_compiling::VmValue, vm_evaluating::VM};

pub mod lexing;
pub mod typing;
pub mod pretty_printing;
pub mod nativelib;
pub mod vm_compiling;
pub mod vm_evaluating;
pub mod parsing;




pub struct ProgramErrorData {
    errors: Vec<ProgramError<ErrType>>,
    warnings: Vec<ProgramError<WarnType>>,
}
impl ProgramErrorData {
    const fn new() -> Self {
        Self { errors: Vec::new(), warnings: Vec::new() }
    }
}
pub struct ProgramSourceData<'a> {
    source_code: &'a str,
    line_lookup: &'a [usize]
}


pub struct ProgramError<T: Display> {
    pub span: Span,
    pub err_type: T,

    // for debugging: where in this source code did the error happen
    pub compiler_location: &'static std::panic::Location<'static>
}

#[derive(Debug, Display, Clone)]
#[display("Error: {_variant}")]
pub enum ErrType {
    #[display("Unexpected character: {c}")]
    LexerUnexpectedCharacter { c: char },
    #[display("Unterminated string.")]
    LexerUnterminatedString,
    #[display("Missing closing brace in template string.")]
    LexerMissingClosingStringBrace,

    #[display("Expected {} {err_msg}. Found '{found}' instead.",
        slice_to_or_string(&expected.iter().map(|x| format!("'{x}'")).collect::<Vec<String>>(), "or")
    )]
    ParserExpectToken { expected: Box<[TokenKind]>, err_msg: String, found: TokenKind },
    #[display("Multiple Expressions on 1 line without a semicolon.")]
    MultipleExprsWithoutSemicolon,
    #[display("Expected an expression. Found {found}")]
    ParserExpectedAnExpression { found: TokenKind },
    #[display("Expected a pattern. Found {found}")]
    ParserExpectedAPattern { found: TokenKind },
    #[display("Expected a binding pattern. Found {found}")]
    ParserExpectedABindingPattern { found: TokenKind },
    #[display("Labels have to be on the same line with the labeled thing.")]
    ParserLabelsHaveToBeOnSameLine,
    #[display("Arrow expressions have to be on the same line with the '=>'.")]
    ParserArrowExprsHaveToBeOnSameLine,
    #[display("Expression is only allowed in statement-position.")]
    ParserOnlyAllowedInStatementPosition,
    #[display("Could not parse number.")]
    ParserNumberParseError,

    #[display("Expected type: {expected}, found: {found}")]
    TyperMismatch { expected: String, found: String, },
    #[display("Undefined identifier: {name}")]
    TyperUndefinedIdentifier { name: String },
    #[display("Can't infer type {}", typ)]
    TyperCantInferType { typ: String, },
    #[display("Type {typ} must be known at this point.")]
    TyperTypeMustBeKnownHere { typ: String, },
    #[display("Pattern doesn't cover all cases. Missing cases: {remaining}")]
    TyperPatternDoesntCoverAllCases { remaining: String },
    #[display("Pattern can't be reached.")]
    TyperPatternCantBeReached,
    #[display("Failable pattern in let-expression. Missing cases: {remaining}")]
    TyperFailableAssignPattern { remaining: String },
    #[display("Requires type annotation.")]
    TyperRequiresTypeAnnotation,
    #[display("Is-expressions that bind variables aren't allowed here.")]
    TyperInvalidBindingIsExpr,
    #[display("break is not allowed outside of loops.")]
    TyperBreakOutsideLoop,
    #[display("could not find the label #{label}. Current labels in scope: {}", available.join(", "))]
    TyperUndefinedLoopLabel { label: String, available: Vec<String> },
    #[display("Expected {expected} arguments, found {found}.")]
    TyperWrongNumberOfArguments { expected: usize, found: usize },
    #[display("Can't call a non-function type: {typ}.")]
    TyperCantCallNonFnType { typ: String, },
    #[display("member .{member} does not exist on type: {typ}")]
    TyperTypeDoesntHaveMember { typ: String, member: String },
    #[display("<never> is not allowed in patterns.")]
    TyperPatternNeverType,
    #[display("All or-patterns must bind the same variables. This pattern binds {}.", slice_to_string(vars, ", "))]
    TyperOrPatternBindsVarsTooMuch { vars: Vec<String> },
    #[display("All or-patterns must bind the same variables. This pattern doesn't bind {}.", slice_to_string(vars, ", "))]
    TyperOrPatternDoesntBindVars { vars: Vec<String> },
    #[display("Not-patterns can't define variables.")]
    TyperNotPatternCantBindVars,
    #[display("String patterns can't have multiple holes right after each other.")]
    TyperPatternStringHolesInARow,
    #[display("Variable {var} cannot be re-assigned, because it isn't declared mutable.")]
    TyperVarIsntDeclaredMut { var: TypeVar },
    #[display("Can't use {var} because it isn't initialized yet.")]
    TyperCantUseUninitializedVar { var: TypeVar },
    #[display("Can't use {var} because it isn't initialized in every possible branch.")]
    TyperCantUseMaybeInitializedVar { var: TypeVar },
    #[display("Can't use {var} because it it was moved in some branches.")]
    TyperCantUseMaybeMovedVar { var: TypeVar },
    #[display("Can't use {var} because it was moved.")]
    TyperCantUseMovedVar { var: TypeVar },
    #[display("Can't dereference non-pointer type: {typ}")]
    TyperCantDerefNonPointerType { typ: String, },
    #[display("Can't deref a non local pointer.")]
    TyperCantDerefUnknownPointerType,
    #[display("Can't index non array type: {typ}")]
    TyperCantIndexNonArrType { typ: String, },
    #[display("Return is only allowed inside functions.")]
    TyperReturnOutsideFunction,
    #[display("Self is only available inside impl-blocks.")]
    TyperSelfOutsideImplBlock,
    #[display("Can't index heterogenous tuple: {typ}")]
    TyperCantIndexHeterogenousTuple { typ: String, },
    #[display("Can't index empty tuple: {typ}")]
    TyperCantIndexEmptyTuple { typ: String, },
    #[display("Must be a customtype, found: {typ}")]
    TyperMustBeCustomtypeType { typ: String, },
    #[display("New-types expect exactly one unlabeled expr.")]
    TyperNewTypesExpectOneUnlabeledExpr,

    #[display("Can't resolve const because it depends on itself.")]
    TyperConstResolvingCycle,
    #[display("Const items can't be mutable.")]
    TyperConstCantBeMutable,
    #[display("A const named {name} already exists in this scope.")]
    TyperConstNameAlreadyExists { name: String },

    #[display("Expected type is not an enum: {typ}. Can't infer the enum variant here.")]
    TyperExpectedTypeIsntAnEnum { typ: String, },
    #[display("Enum {enum_} doesn't have variant: .{variant}")]
    TyperEnumDoesntHaveVariant { enum_: String, variant: Box<str> },
    #[display("Enum variant .{variant} requires data.")]
    TyperVariantRequiresData { variant: String },
    #[display("Expected exact variant .{variant}, found .{found}")]
    TyperEnumExpectedExactVariant { variant: String, found: String, },

    #[display("Runtime values aren't allowed in impl-blocks.")]
    TyperRuntimeValuesArentAllowedInImplBlocks,
    #[display("Expected a const, found a runtime value: {name}")]
    TyperExpectedConstFoundRuntimeValue { name: String },
    #[display("impl can only be used on custom types, found: {typ}")]
    TyperCantImplNonCustomType { typ: String },

    // #[display("Can't borrow because already borrowed mutably.")]
    // TyperCantBorrowBecauseAlreadyBorrowedMut,
    // #[display("Can't borrow mutably because already borrowed.")]
    // TyperCantBorrowMutBecauseAlreadyBorrowed,
    // #[display("Can't borrow mutably because already borrowed mutably.")]
    // TyperCantBorrowMutBecauseAlreadyBorrowedMut,

    #[display("RuntimeError: {msg}")]
    RuntimeError { msg: String },


    // this case should not be used, every error should have its own entry in this enum!
    #[display("{_0}")]
    DefaultString(String),
}



#[derive(Debug, Display, Clone)]
#[display("Warning: {_variant}")]
pub enum WarnType {
    #[display("Incosistent spacing around infix {op}")]
    ParserInconsistentSpacingAroundInfixOp { op: TokenKind }
}






pub fn run_code(source_code: &str) -> Result<VmValue, Vec<ErrType>> {
    // line numbers are gonna be messed up by just slapping the prelude before the code
    // but i don't care for now
    const INCLUDE_PRELUDE: bool = true;
    const PRELUDE: &str = if INCLUDE_PRELUDE { include_str!("prelude.thrum") } else { "" };
    let preluded_source_code = &format!("{PRELUDE}\n{source_code}");

    let mut err_data = ProgramErrorData::new();

    let (lexer_tokens, line_lookup) = lexing::Lexer::start(&mut err_data, preluded_source_code);
    let source_data = ProgramSourceData { source_code: preluded_source_code, line_lookup: &line_lookup };
    stage_complete("Lexer", &slice_to_string(&lexer_tokens, ", "), &err_data, &source_data)?;

    let mut ast = parsing::Parser::start(&mut err_data, preluded_source_code, &lexer_tokens);
    drop(lexer_tokens);
    stage_complete("Parser", &ast.display_expr(ExprId(0)), &err_data, &source_data)?;

    parsing::desugar::desugar_after_parsing(&mut ast);
    stage_complete("Desugar", &ast.display_expr(ExprId(0)), &ProgramErrorData::new(), &source_data)?;

    let (typed_ast, mut compiled_functions) = typing::TypeChecker::start(&mut err_data, &ast);
    stage_complete("Typechecker", &format!("compiled_functions: {compiled_functions:?}"), &err_data, &source_data)?;

    vm_compiling::VmCompiler::start(&ast, &typed_ast, &mut compiled_functions);
    stage_complete("VmCompiler", &format!("{compiled_functions:?}"), &ProgramErrorData::new(), &source_data)?;


    let start_execution_time = Instant::now();
    let result = unsafe { VM::start(&mut compiled_functions, None) };
    match result {
        Ok(r) => {
            println!("\n--- Execution Successfull ({:?}) ---", start_execution_time.elapsed());
            println!("{r}");
            Ok(r)
        }
        Err(err) => {
            println!("\n--- Runtime Error ({:?}) ---", start_execution_time.elapsed());
            println!("{err}");
            Err(vec![err])
        }
    }
}


fn stage_complete(stage: &str, msg: &str, err_data: &ProgramErrorData, source_data: &ProgramSourceData) -> Result<(), Vec<ErrType>> {
    println!("--- {stage} complete! ---");
    if !msg.is_empty() {
        println!("{msg}\n");
    }

    if !err_data.warnings.is_empty() {
        println!("--- WARNINGS ---\n{}", err_data.warnings.iter().map(|e| format_program_error(e, source_data) + "\n").collect::<String>());
    }

    if err_data.errors.is_empty() {
        Ok(())
    } else {
        println!("--- Errors ---\n{}", err_data.errors.iter().map(|e| format_program_error(e, source_data) + "\n").collect::<String>());
        Err(err_data.errors.iter().map(|x| x.err_type.clone()).collect())
    }
}