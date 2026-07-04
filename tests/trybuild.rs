#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
    // D4 regression: the SAME return-`impl Trait` shape that aborts under (default-on)
    // `support_infinite_cycle` must still compile fine when it's turned off (`emit_reentry_items`,
    // and its abort, only runs under `support_infinite_cycle`).
    t.pass("tests/ui/pass/*.rs");
}
