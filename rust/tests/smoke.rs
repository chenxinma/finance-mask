// rust/tests/smoke.rs
#[test]
fn crate_builds_and_version_constant_exists() {
    assert_eq!(finance_mask_core::VERSION, "0.1.0");
}
