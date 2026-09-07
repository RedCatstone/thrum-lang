#![allow(clippy::needless_raw_string_hashes)]
pub use thrum_engine::run_code;

#[macro_export] macro_rules! test {
    ($code:expr, $expected:expr) => {
        match common::run_code($code).0 {
            Ok(val) => assert_eq!(val, $expected),
            Err(errs) => panic!("Code failed to compile!\nErrors: {errs:#?}"),
        }
    };
}

#[macro_export] macro_rules! test_err {
    ($code:expr, $err_pat:pat) => {
        match common::run_code($code).0 {
            Ok(val) => panic!("Code should have failed, but it successfully evaluated to: {val:?}"),
            Err(errs) => assert!(
                errs.iter().any(|e| matches!(e, $err_pat)),
                "Expected error matching {}, instead found errors:\n{:?}", stringify!($err_pat), errs
            )
        }
    };
}


#[macro_export] macro_rules! test_warn {
    ($code:expr, $warn_pat:pat) => {
        let (result, warns) = common::run_code($code);
        assert!(
            warns.iter().any(|w| matches!(w, $warn_pat)),
            "Expected warning matching {}, instead found warnings:\n{warns:?}\nResult was: {result:?}",
            stringify!($warn_pat)
        );
    };
}

#[macro_export] macro_rules! test_no_warn {
    ($code:expr) => {
        let (result, warns) = common::run_code($code);
        assert!(
            warns.is_empty(),
            "Expected no warnings, instead found:\n{warns:?}\nResult was: {result:?}"
        );
    };
}