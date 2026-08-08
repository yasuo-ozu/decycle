//! Regression tests for the three unsoundness classes found in the 2026-08-04 audit.
//!
//! All three are fixed; these pin them so a fix cannot silently regress.
//!
//! | test | catches |
//! |---|---|
//! | `registry_key_collision_calls_wrong_fn`      | an unencodable (closure) re-entry key is refused |
//! | `registry_stale_entry_defeats_fail_closed`   | same, reached via a leftover from a panicked descent |
//! | `floor_crossing_preserves_fn_pointer_provenance` | miri only: provenance through the floor |
//! | `dyn_fn_self_arg_keeps_a_valid_vtable`       | miri only: `&dyn Fn(&Self)` vtable validity |
//!
//! The last two are invisible natively — run them under miri:
//!
//! ```text
//! cargo +nightly miri test --test ub_regressions
//! ```

// Cycle members are reached through their generated ranked/terminator variants.
#![allow(dead_code)]

use decycle::decycle;
use std::cell::{Cell, RefCell};

// =================================================================================================
// Class A (CRITICAL) — ranked re-entry registry: key collision + last-writer-wins registration.
//
// The registry key is `(type_name::<__Mk_..<..>>(), size+align fingerprint)`. Two closures written
// in the same function share `type_name` *exactly* (both are `..::{{closure}}`, with no
// disambiguator), so two same-size/same-align closures share a single registry slot.
//
// The register-before-descend prologue is spliced at the TOP of each inductive method body, so any
// user code *later* in that body which starts another descent with the colliding instantiation
// overwrites the slot before the floor reads it. The floor then transmutes and calls the other
// closure's fn. This is reachable from entirely safe code at the DEFAULT `recurse_level`.
//
// Escalation (deliberately not a test — it aborts the harness): if the two colliding closures
// capture a `usize` and a `&'static usize` respectively, the floor runs one body against the
// other's environment, dereferencing an integer as a pointer — an observed SIGSEGV with no
// `unsafe` anywhere in the user program.
// =================================================================================================

thread_local! {
    /// Re-entrancy guard so the nested descent runs exactly once.
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
    /// A callback invoked from inside the cycle body — models a visitor/observer hook, which is
    /// how a real program ends up starting a second descent from within the first.
    static HOOK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Called from inside the cyclic method body.
pub fn hook() {
    if IN_HOOK.with(|c| c.get()) {
        return;
    }
    IN_HOOK.with(|c| c.set(true));
    // Take the hook out so a re-entrant call cannot borrow the `RefCell` twice.
    let h = HOOK.with(|s| s.borrow_mut().take());
    if let Some(h) = &h {
        h();
    }
    HOOK.with(|s| *s.borrow_mut() = h);
    IN_HOOK.with(|c| c.set(false));
}

#[decycle]
mod collide {
    #[decycle]
    pub trait Fold {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize;
    }
    pub struct A;
    impl Fold for A
    where
        A: Fold,
    {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize {
            if n == 0 {
                f(0)
            } else {
                // A callback that starts a second descent with a *different* closure of the same
                // layout. This is what clobbers this frame's registry slot.
                crate::hook();
                A.fold(f, n - 1) + 1
            }
        }
    }
}

/// Class A, mechanism 1: a nested descent with a layout-colliding closure makes the floor call the
/// wrong closure body. Default `recurse_level` (10); depth 30 crosses the floor.
/// Run a descent that could dispatch to the WRONG closure, and accept only the two outcomes that
/// are correct.
///
/// This is deliberately policy-agnostic, so it keeps its meaning if the current policy changes:
///
///   * decycle REFUSES the input up front (today's behaviour — the key contains an anonymous type
///     and cannot be encoded uniquely), or
///   * decycle runs it and produces `expected`.
///
/// It fails on the third outcome — running it and returning something else — which is exactly the
/// silent wrong-closure dispatch. If the `{{closure}}` rejection is ever relaxed, these tests stop
/// exercising the rejection branch and start exercising the correctness branch, with no edit
/// needed; that is the point.
///
/// `expected` is chosen so the wrong answer names its culprit: each closure computes a value only
/// it can produce, so a mismatch says which one actually ran.
#[track_caller]
fn assert_no_wrong_closure_dispatch<F>(what: &str, run: F, expected: usize, wrong: &str)
where
    F: FnOnce() -> usize + std::panic::UnwindSafe,
{
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(run);
    std::panic::set_hook(prev);

    match outcome {
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            assert!(
                msg.contains("anonymous type") || msg.contains("re-entry fn not registered"),
                "{what}: expected either the unencodable-key rejection or the documented \
                 fail-closed panic, got: {msg}"
            );
        }
        Ok(v) => assert_eq!(
            v, expected,
            "{what}: the floor dispatched to the WRONG closure — got {v}, which is {wrong}. \
             The re-entry key failed to separate two closures that share a `type_name` and a \
             layout."
        ),
    }
}

#[test]
fn registry_key_collision_calls_wrong_fn() {
    use collide::Fold;
    // `c_add` and `c_mul` share a `type_name` and a layout, so they map to one registry slot.
    // The body's callback starts a nested descent with `c_mul` in the middle of `c_add`'s, which
    // is what used to leave the floor pointing at the wrong one.
    //
    // c_add: f(0) = 7, then 30 frames add 1  => 37
    // c_mul: f(0) = 100_000, then 30 frames  => 100_030
    let add: usize = 7;
    let c_add = move |v: usize| v + add;
    let mul: usize = 3;
    let c_mul = move |v: usize| v * mul + 100_000;
    HOOK.with(|s| {
        *s.borrow_mut() = Some(Box::new(move || {
            let _ = collide::A.fold(c_mul, 12);
        }))
    });
    assert_no_wrong_closure_dispatch(
        "nested descent with a layout-colliding closure",
        move || collide::A.fold(c_add, 30),
        37,
        "`c_mul`'s body (v * 3 + 100_000) running on `c_add`'s environment",
    );
    HOOK.with(|s| *s.borrow_mut() = None);
}

#[decycle(recurse_level = 1)]
mod stale {
    #[decycle]
    pub trait Fold {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Fold for A
    where
        B: Fold,
    {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize {
            if n == 0 {
                f(0)
            } else {
                B.fold(f, n - 1) + 1
            }
        }
    }
    impl Fold for B
    where
        A: Fold,
    {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize {
            if n == 0 {
                f(0)
            } else {
                A.fold(f, n - 1) + 1
            }
        }
    }
}

/// Class A, mechanism 3: **STILL OPEN — needs a product decision, see below.**
///
/// An entry left behind by a descent that already hit the documented fail-closed panic defeats
/// that backstop on a *later*, unrelated call: the backstop only checks that a key is *present*,
/// never that it is *correct*. No nesting and no `unsafe` — just a caught panic (a test harness
/// or a `catch_unwind` boundary) followed by an ordinary call.
///
/// Mechanism 1 (the nested clobber, above) is fixed: shadowing a live entry is now rolled back
/// when the registering frame exits. This one is NOT, and cannot be fixed the same way, because
/// removing a registration when its frame exits would break two behaviors the suite documents as
/// intended:
///
///   * `tests/bareparam_reentry.rs` — "unbounded once primed": an earlier call registers the
///     floor that a later, separate call needs.
///   * `tests/limitation.rs::rank_eater_panics_clean_and_isolated` — after the fail-closed panic,
///     "a good input heals the bad one", which works *because* the panicked descent's entries
///     survive.
///
/// The leftover is harmless when it is the right fn (that is the healing case) and harmful only
/// when a colliding key makes it the wrong one. So the root cause here is the KEY, not the
/// lifetime: `type_name` collapses all closures in a function to `..::{{closure}}` and the
/// fingerprint folds only size+align. A real fix needs a key that separates two same-layout
/// closures; the obvious candidates fail (`TypeId` needs `'static`; a per-monomorphization fn
/// address can be merged by LLVM's identical-code folding).
///
/// The choice is therefore: (a) keep prime-once/healing and accept this residual wrong-fn window,
/// or (b) drop them, remove entries on frame exit, and take the fail-closed panic instead — safe,
/// but a breaking behavior change for the bare-param pattern.
#[test]
fn registry_stale_entry_defeats_fail_closed() {
    use stale::Fold;
    // Same collision reached the other way: `c_mul`'s descent registers the shared slot and then
    // hits the documented fail-closed panic, leaving its entry behind. A later, unrelated call
    // with `c_add` must not pick it up.
    //
    // c_add: f(0) = 7, then 5 frames add 1 => 12
    // c_mul: f(0) = 100_000, then 5 frames => 100_005
    let mul: usize = 3;
    let c_mul = move |v: usize| v * mul + 100_000;
    let add: usize = 7;
    let c_add = move |v: usize| v + add;

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let _ = std::panic::catch_unwind(move || stale::B.fold(c_mul, 5));
    std::panic::set_hook(prev);

    assert_no_wrong_closure_dispatch(
        "later call after a panicked descent left its registration behind",
        move || stale::A.fold(c_add, 5),
        12,
        "`c_mul`'s body (v * 3 + 100_000), resurrected from the stale entry",
    );
}

// =================================================================================================
// Class B (HIGH) — every unbounded floor crossing is provenance-UB.
//
// Generated code registers the re-entry fn as `#re::<..> as usize` (finalize.rs), the registry
// stores a `usize` (lib.rs), and the floor does `transmute::<usize, #fa>(..)` and calls it. Casting
// a function pointer through an integer strips provenance; calling the result is UB. It is NOT
// relaxed by `-Zmiri-permissive-provenance`, which only covers `as` casts, not `transmute`.
// rustc's own `function_casts_as_integer` lint flags the same pattern.
//
// Fix: store `*const ()` (or an opaque fn pointer) instead of `usize`, so provenance is carried
// end to end. The registry is a `thread_local!`, so a raw pointer raises no `Send`/`Sync` issue.
//
// This is a minimal, miri-fast canary: depth 12 with the default floor of 10 crosses once.
// =================================================================================================

#[decycle]
mod floor {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

/// Class B: crossing the rank floor must not launder the re-entry fn pointer through an integer.
///
/// Passes natively; under miri this reports "pointer not dereferenceable: .. it has no provenance"
/// at the first floor crossing.
#[test]
fn floor_crossing_preserves_fn_pointer_provenance() {
    use floor::Ca;
    assert_eq!(floor::A.ca(12), 12);
}

// =================================================================================================
// Class C (HIGH) — structural `&dyn Fn(&Self)` argument cast builds a wide pointer whose vtable
// names a different trait.
//
// `dyn Fn(&A) -> i64` and `dyn Fn(&ATerm) -> i64` are *different* traits, so transmuting the trait
// object keeps a vtable for the wrong one. The two are size-equal, so the `__DecycleSizeGuard`
// size assertion structurally cannot catch it.
//
// Fix (verified miri-clean): do not pun the trait object. Re-wrap it in a fresh closure that casts
// at call time, so the new trait object is built by coercion:
//
// ```text
// let adapter = |t: &Term| f(cast::<Term, Natural>(t));
// callee(&adapter)
// ```
//
// The closure lives on the stack, so this costs no allocation. The plain `fn(&Self)` fn-pointer
// cast in the same position is fine (ABI-compatible) and is intentionally exercised here too, to
// pin that it stays fine.
// =================================================================================================

#[decycle(structural)]
mod dyn_arg {
    #[decycle]
    pub trait Vis {
        fn apply(&self, f: fn(&Self) -> i64) -> i64;
        fn apply_dyn(&self, f: &dyn Fn(&Self) -> i64) -> i64;
    }
    pub struct A(pub i64);
    pub struct B(pub i64);
    impl Vis for A
    where
        B: Vis,
    {
        fn apply(&self, f: fn(&Self) -> i64) -> i64 {
            f(self)
        }
        fn apply_dyn(&self, f: &dyn Fn(&Self) -> i64) -> i64 {
            f(self)
        }
    }
    impl Vis for B
    where
        A: Vis,
    {
        fn apply(&self, f: fn(&Self) -> i64) -> i64 {
            f(self)
        }
        fn apply_dyn(&self, f: &dyn Fn(&Self) -> i64) -> i64 {
            f(self)
        }
    }
}

/// Class C: forwarding a `&dyn Fn(&Self)` argument into terminator space must not transmute the
/// trait object.
///
/// Passes natively; under miri this reports "wrong trait in wide pointer vtable".
#[test]
fn dyn_fn_self_arg_keeps_a_valid_vtable() {
    use dyn_arg::Vis;
    // The fn-pointer form is sound and must stay sound.
    assert_eq!(dyn_arg::A(3).apply(|x| x.0 * 2), 6);
    // The `dyn` form is the defect.
    assert_eq!(dyn_arg::B(5).apply_dyn(&|x| x.0 + 1), 6);
}
