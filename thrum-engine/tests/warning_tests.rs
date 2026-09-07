use thrum_engine::WarnType;
mod common;


#[test] fn normally_no_warn() {
    test_no_warn!("1 + 1");
}

#[test] fn unused_var_warn() {
    test_warn!("let x = 5", WarnType::UnusedVar { .. });
    test_no_warn!("let _x = 5");
    test_no_warn!("let x = 5; x^");
}

#[test] fn unused_mut_var_warn() {
    test_warn!("let mut x = 5; x^", WarnType::UnusedMutVar { .. });
    test_warn!("let mut _x = 5; _x^", WarnType::UnusedMutVar { .. });
    test_no_warn!("let mut x = 5; x = 10; x^");
}

#[test] fn inconsistent_spacing_warn() {
    test_warn!("1+ 2", WarnType::ParserInconsistentSpacingAroundInfixOp { .. });
    test_warn!("1 +2", WarnType::ParserInconsistentSpacingAroundInfixOp { .. });
    test_warn!("1  + 2", WarnType::ParserInconsistentSpacingAroundInfixOp { .. });

    test_no_warn!("1  +  2");
    test_no_warn!("1 + 2");
    test_no_warn!("1+2");
}