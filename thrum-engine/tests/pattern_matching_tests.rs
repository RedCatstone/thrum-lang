use thrum_engine::{ErrType, vm_compiling::VmValue};
mod common;

// =========================================================================
// Basic Matching
// =========================================================================
#[test] fn basic_int_pattern() {
    test!("7 is 7", VmValue::Bool(true));
    test!("7 is 0", VmValue::Bool(false));
}
#[test] fn basic_float_pattern() {
    test!("3.14 is 3.14", VmValue::Bool(true));
    test!("3.14 is 2.71", VmValue::Bool(false));
}
#[test] fn basic_bool_pattern() {
    test!("true is true", VmValue::Bool(true));
    test!("true is false", VmValue::Bool(false));
}
#[test] fn basic_wildcard() {
    test!("0 is _", VmValue::Bool(true));
    test!("0.1 is _", VmValue::Bool(true));
    test!("\"hello\" is _", VmValue::Bool(true));
    test!("true is _", VmValue::Bool(true));
}

#[test] fn is_expr_precedence() {
    test!("1 + 1 is 1 * 1 or 2 + 2 is 2 * 2 and 3 + 3 is !3 * 3 and 4 + 4 is !4 * 4", VmValue::Bool(true));
}

#[test] fn chained_conditional_pattern_bindings() {
    test!("5 is let x and 10 is let z and 100 is let y", VmValue::Bool(true));
    test!("if 5 is let x and 10 is let z and 100 is let y => x + z + y else 0", VmValue::Int(115));
    test!("if 5 is let x and 10 is 99 and 100 is let y => x + y else 0", VmValue::Int(0));
}

#[test] fn basic_or_pattern() {
    test!("7 is 5 | 6 | 7", VmValue::Bool(true));
    test!("5 is 5 | 6 | 7", VmValue::Bool(true));
    test!("0 is 5 | 6 | 7", VmValue::Bool(false));
}
#[test] fn basic_not_pattern() {
    test!("7 is !0", VmValue::Bool(true));
    test!("7 is !7", VmValue::Bool(false));
    test!("0 is !_", VmValue::Bool(false));
}
#[test] fn not_pattern_with_or() {
    test!(r#"5 is !(4 | 5) and panic("should've failed")"#, VmValue::Bool(false));
    test!(r#"5 is !(4 | 6) or panic("should've matched")"#, VmValue::Bool(true));
    test!("7 is !7 | !6", VmValue::Bool(true));
    test!("0 is !7 | !6", VmValue::Bool(true));
}

#[test] fn basic_conditional_pattern() {
    test!("let num = 15; num is let x and x > 10", VmValue::Bool(true));
    test!("let num =  5; num is let x and x > 10", VmValue::Bool(false));
}


// =========================================================================
// Pointer Matching
// =========================================================================
#[test] fn ref_int_matching() {
    test!("let x = 5; x is 1 | 2 | 5", VmValue::Bool(true));
    test!("let x = 5; x is 9 | 99", VmValue::Bool(false));
    test!("let x = 5; x is !0", VmValue::Bool(true));
    test!("let x = 5; x is !5", VmValue::Bool(false));
    test!("let x = 5; if x is 5 => 100 else -1", VmValue::Int(100));
}
#[test] fn ref_bool_matching() {
    test!("let flag = true; flag is true", VmValue::Bool(true));
    test!("let flag = true; flag is !false", VmValue::Bool(true));
}
#[test] fn ref_float_matching() {
    test!("let pi = 3.14; pi is 3.14", VmValue::Bool(true));
}
#[test] fn ref_str_matching() {
    test!(r#"let s = "cat"; s is "cat" "#, VmValue::Bool(true));
    test!(r#"let s = "cat"; s is "bad" "#, VmValue::Bool(false));

    test!(r#"let x = "pleh"; let y = "argh"; x is y^ "#, VmValue::Bool(false));
}
#[test] fn ref_tuple_matching() {
    test!("let x = (69, 420); x is (69, let y and y > 400)", VmValue::Bool(true));
}

#[test] fn double_ref_compare_pattern() {
    test!("let x = 5; let x_ref = x; x_ref is 5", VmValue::Bool(true));
    test!("let x = 4; let x_ref = x; x_ref is 5", VmValue::Bool(false));

    test!("let x = 5; if x is let y => y is 5 else false", VmValue::Bool(true));
}
#[test] fn alot_ref_compare_pattern() {
    test!("
        fn ref5(p: &&&&&int) -> bool => p is 5
        fn ref4(p: &&&&int) -> bool => ref5(p)
        fn ref3(p: &&&int) -> bool => ref4(p)
        fn ref2(p: &&int) -> bool => ref3(p)
        fn ref1(p: &int) -> bool => ref2(p)
        let x = 5
        ref1(x)
    ", VmValue::Bool(true));
}

#[test] fn ref_in_match_arms() {
    test!("
        let x = 5
        match x
        is 1 => 10
        is 5 => 50
        is _ => 0
    ", VmValue::Int(50));
}




// =========================================================================
// Tuple, String, Enum Patterns
// =========================================================================
#[test] fn tuple_exact_match() {
    test!("(2,) is (2,)", VmValue::Bool(true));
    test!("(2, 1) is (2, 1)", VmValue::Bool(true));
    test!("(x: 2) is (x: 2)", VmValue::Bool(true));
    test!("(x: 2, y: 3) is (x: 2, y: 3)", VmValue::Bool(true));

    test!("(1, 2) is (1, 1) | (1, 2) | (1, 3)", VmValue::Bool(true));
    test!("(1, 2) is (1, 1) | (1, 3)", VmValue::Bool(false));

    test!("(1, 2) is (1 | 2, 3 | 4)", VmValue::Bool(false));
    test!("(2, 3) is (1 | 2, 3 | 4)", VmValue::Bool(true));
}

#[test] fn tuple_destructure() {
    test!("
        let tup = ((1, 2), 2, (3, 4, 5))
        let (_, a, (_, b, _)) = tup
        a * b
    ", VmValue::Int(8));

    test!(r#"
        let status = "ok"
        let data = (id: 42, status:)

        if data is (status: "ok", id:) {
            id^^
        } else {
            -1
        }
    "#, VmValue::Int(42));

    test!("
        let (x, true | false) = (3, true)
        x^
    ", VmValue::Int(3));
}


#[test] fn basic_string_pattern() {
    test!(r#" "" is "" "#, VmValue::Bool(true));
    test!(r#" "a" is "" "#, VmValue::Bool(false));
    test!(r#" "" is "a" "#, VmValue::Bool(false));

    test!(r#" "" is "{_}" "#, VmValue::Bool(true));
    test!(r#" "a" is "{_}" "#, VmValue::Bool(true));

    test!(r#" "" is "{""}" "#, VmValue::Bool(true));
    test!(r#" "a" is "{""}" "#, VmValue::Bool(false));
}

#[test] fn string_pattern_search() {
    test!(r#" "Pat the Cat likes to sit on a Mat." is "{_}Cat{_}" "#, VmValue::Bool(true));
    test!(r#" "Pat the Dog likes to sit on a Log." is "{_}Cat{_}" "#, VmValue::Bool(false));
    test!(r#" "Pat the Dog likes to sit on a Log." is "{_} Dog {_}" "#, VmValue::Bool(true));
}

#[test] fn string_pattern_destructure_holes() {
    test!(r#"
        if "hello cat from Earth!" is let "hello {name} from {location}!" => (name, location)
        else panic("")
    "#, VmValue::Tup(vec![VmValue::OwnedStr("cat".to_string()), VmValue::OwnedStr("Earth".to_string())]));

    test!(r#"  "hello" is "h{"e{"l"}"}lo"  "#, VmValue::Bool(true));

    // kinda annoying that this returns false, but technically it makes sense.
    // it fails because the first hole gets `""` and compares that to `"l"`
    test!(r#"  "ll" is "{"l"}l{_}"  "#, VmValue::Bool(false));
}

#[test] fn string_pattern_multiple_consecutive_holes() {
    // TODO: ...
    // test!(r#"
    //     if "<1, 2, 3>" is let "<{num1}, {num2}, {num3}>" => (num1, num2, num3)
    //     else "nop"
    // "#, VmValue::OwnedStr("123".to_string()));
}



#[test] fn basic_enum_pattern() {
    // TODO: remove ^ in enum matching
    test!("
        let x: Option = :None
        x is :None
    ", VmValue::Bool(true));

    test!("
        const Dir = enum { Up, Down{ bool, bool } }
        let up: Dir = :Up
        up is :Down{ _, _ }
    ", VmValue::Bool(false));
}

#[test] fn enum_pattern_with_data() {
    test!("Option.Some{ 3 } is :Some{ 3 }", VmValue::Bool(true));
    test!("Option.Some{ 3 } is :Some{ 2 }", VmValue::Bool(false));
    test!("let x = Option.Some{ 3 }; x is :Some{ 3 }", VmValue::Bool(true));
    test!("let x = Option.Some{ 3 }; x is :Some{ 2 }", VmValue::Bool(false));
    test!("let x = Option.Some{ 3 }; let x = x; x is :Some{ 3 }", VmValue::Bool(true));

    test!("
        const Dir = enum { Up, Down{ bool, bool } }
        let down: Dir = :Down{ true, false }
        down is :Down{ true, !true }
    ", VmValue::Bool(true));
}


#[test] fn new_type_pattern() {
    test!("
        type NewInt = int
        let x = NewInt{ 3 }
        let x = x
        let NewInt{ inner } = x  // preserves all pointers
        inner^^^
    ", VmValue::Int(3));

    test!("
        type Point = (x: int, y: int)
        let p = Point{ x: 10, y: 20 }
        p is Point{ x: 10, y: let y } and y == 20
    ", VmValue::Bool(true));
}


#[test] fn pattern_exhaustiveness() {
    test!("let true | false = true", VmValue::Void);
}


// =========================================================================
// Match Expressions
// =========================================================================
#[test] fn match_expr() {
    test!(r#"
        match (69, "yay!")
        is (69, "") => "nope1"
        is (0, "yay!") => "nope2"
        is (0, "") => "nope3"
        is (69, let x) => x^
        is _ => "nope4"
    "#, VmValue::OwnedStr("yay!".to_string()));

    test!("
        let val: Option = :Some{ 42 }
        match val^
        is :None => -1
        is :Some{ 0 | 1 | 2 } => 0
        is :Some{ !42 } => 1
        is :Some{ let x } => x^
    ", VmValue::Int(42));
}

#[test] fn match_expr_single_line() {
    test!("match false is true => 0, is _ => -3", VmValue::Int(-3));
    test!("match () is () => ()", VmValue::Tup(vec![]));
}


// =========================================================================
// Pattern Errors
// =========================================================================
#[test] fn err_pattern_type_mismatch() {
    test_err!("1 is \"hello\"", ErrType::TyperMismatch { .. });
    test_err!("1 is true", ErrType::TyperMismatch { .. });
    test_err!("true is 1", ErrType::TyperMismatch { .. });
    test_err!("let x = 5; x is true", ErrType::TyperMismatch { .. });

    test_err!("(1, 2) is (1, \"two\")", ErrType::TyperMismatch { .. });
    test_err!("(1, 2) is 1 | 2", ErrType::TyperMismatch { .. });
    test_err!("(1, 2) is (1, 2, 3)", ErrType::TyperMismatch { .. });
    test_err!("(1, 2) is (1,)", ErrType::TyperMismatch { .. });
}

#[test] fn err_pattern_exhaustiveness() {
    test_err!("match 5 is 1 => 2", ErrType::TyperPatternDoesntCoverAllCases { .. });
    test_err!("let x = Option.Some{ 1 }; match x^ is :None | :Some{ 1 } => 0", ErrType::TyperPatternDoesntCoverAllCases { .. });
    test_err!("match (true, false) is (true, true) => 0, is (false, _) => 1", ErrType::TyperPatternDoesntCoverAllCases { .. });

    test_err!("let true = true", ErrType::TyperFailableAssignPattern { .. });
    test_err!("let (x, 0) = (1, 2)", ErrType::TyperFailableAssignPattern { .. });
    // TODO: test_warn!("match 5 is _ => 1 \n is 5 => 2", WarnType::TyperPatternCantBeReached);
}

#[test] fn err_pattern_binding() {
    test_err!("let a = (5 is let x)", ErrType::TyperInvalidBindingIsExpr);
    test_err!("if (5 is let x) is true {}", ErrType::TyperInvalidBindingIsExpr);

    test_err!("5 is !(let x)", ErrType::TyperNotPatternCantBindVars);

    test_err!("if (1, 2) is let (x, y) | (x, _) {}", ErrType::TyperOrPatternDoesntBindVars { .. });
    test_err!("if (1, 2) is let (x, _) | (x, y) {}", ErrType::TyperOrPatternBindsVarsTooMuch { .. });
    test_err!("if (1, 2) is let (x, x) {}", ErrType::TyperPatternNameAlreadyBound { .. });

    test_err!("if (1, 2) is let (x, 0) | (0, y) { }", ErrType::TyperOrPatternDoesntBindVars { .. });
    test_err!("let (x, 0) = (1, 2)", ErrType::TyperFailableAssignPattern { .. });
}

#[test] fn err_pattern_string_holes() {
    test_err!(r#" "abc" is "a{_}{_}c" "#, ErrType::TyperPatternStringHolesInARow);
    test_err!(r#" "abc" is "{}" "#, ErrType::ParserExpectedAPattern { .. });
}