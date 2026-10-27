use thrum_engine::vm_compiling::VmValue;
mod common;


#[test] fn expected_type_variables() {
    test!("
        let mut x: Option = :None
        x = :Some{ 3 }
        x^ is !:None
    ", VmValue::Bool(true));
}

#[test] fn expected_type_tuples() {
    test!("
        let tup: (Option, Option) = (:None, :Some{ 4 })
        tup.0^ is :None
    ", VmValue::Bool(true));

    test!("
        fn make_arr() -> (Option; 2) => (:None, :Some{ 1 })
        make_arr().0 is :None
    ", VmValue::Bool(true));
}

#[test] fn expected_type_tuple_patterns() {
    test!("(Option.None, Option.Some{ 4 }) is (:None, :Some{ 4 })", VmValue::Bool(true));
    test!("(Option.None; 2) is (:None, :None)", VmValue::Bool(true));
}

#[test] fn expected_type_blocks() {
    test!("
        let x: Option = {
            let y = 5;  // TODO: fix this semicolon
            :Some{ y^ }
        }
        x^ is :Some{ 5 }
    ", VmValue::Bool(true));
}

#[test]
#[expect(clippy::literal_string_with_formatting_args, reason="kinda funny that this lints on thrum code ;p")]
fn expected_type_control_flow() {
    test!("
        let x: Option = if false { :None } else { :Some{ 99 } }
        x^ is :Some{ 99 }
    ", VmValue::Bool(true));

    test!("
        let x: Option = match 42
            is 0 => :None
            is _ => :Some{ 42 }
        x^ is :Some{ 42 }
    ", VmValue::Bool(true));
}

#[test] fn expected_type_functions() {
    test!("
        fn test_opt(val: Option) -> bool {
            val^ is :None
        }
        test_opt(:None)
    ", VmValue::Bool(true));

    test!("
        fn get_none() -> Option {
            return :None
        }
        get_none() is :None
    ", VmValue::Bool(true));

    // test!("
    //     const Option = enum { None, Some(int) }
    //     let func: |int| -> Option = |n| => :Some(n^)
    //     func(7) is :Some(7)
    // ", VmValue::Bool(true));
}

#[test] fn expected_type_breaks() {
    test!("
        let x: Option = loop {
            break :None
        }
        x^ is :None
    ", VmValue::Bool(true));

    test!("
        let x: Option = { #bloc
            break #bloc :None
        }
        x^ is :None
    ", VmValue::Bool(true));
}