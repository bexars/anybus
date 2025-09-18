#[test]
fn test_handle_builder_compile_fails() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/builder/test1.rs");
}
