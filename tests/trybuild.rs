//! Split deliberately in two, because the two halves have very different failure modes.
//!
//! `ui_pass` compiles programs that must build. There is no `.stderr` snapshot involved, so a new
//! rustc release cannot legitimately "drift" it — a failure here is a real regression. It stays a
//! normal test so the required CI job runs it. It matters: `tests/ui/pass/value_generic_no_growth.rs`
//! is the only thing pinning the growth heuristic against *false positives*, and for a while it
//! could not fail CI at all.
//!
//! `ui_compile_fail` compares rustc's diagnostics against blessed `.stderr` files, which are
//! genuinely version-sensitive. It is `#[ignore]`d so the floating-stable job skips it, and a
//! dedicated CI job runs it on a pinned toolchain with `--ignored`. That replaces the old
//! `--skip ui` filter, which matched on a substring and would silently have skipped any future
//! test whose name merely contained "ui".
//!
//! Run it locally with the pinned toolchain named in `.github/workflows/ci.yml`:
//!
//! ```text
//! cargo +1.96.0 test --test trybuild -- --ignored
//! ```
//!
//! To re-bless after an intentional diagnostic change:
//!
//! ```text
//! TRYBUILD=overwrite cargo +1.96.0 test --test trybuild -- --ignored
//! ```

#[test]
fn ui_pass() {
    let t = trybuild::TestCases::new();
    // D4 regression: the SAME return-`impl Trait` shape that aborts under (default-on)
    // `support_infinite_cycle` must still compile fine when it's turned off (`emit_reentry_items`,
    // and its abort, only runs under `support_infinite_cycle`).
    t.pass("tests/ui/pass/*.rs");
}

#[test]
#[ignore = "snapshot-sensitive; run on the pinned toolchain via the `ui` CI job or `-- --ignored`"]
fn ui_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
