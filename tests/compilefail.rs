//! Compile-fail tests for `implement!` validation and `pattern!` structural
//! checks (plans/milestone-1 task 8). Errors quote the offending tokens in
//! the message text: spans have degraded to the call site by the time the
//! checks run.

#[test]
fn compilefail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compilefail/*.rs");
}
