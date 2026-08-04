//! Regression tests for the three unsoundness classes found in the 2026-08-04 audit.
//!
//! **These tests are expected to FAIL until the underlying defects are fixed.** They exist to
//! pin the defects so a fix can be verified and can never silently regress. Two of them fail
//! under plain `cargo test`; two reproduce undefined behavior that only miri observes, so they
//! *pass* natively today and must be run under miri to be meaningful:
//!
//! | test | native | miri |
//! |---|---|---|
//! | `registry_key_collision_calls_wrong_fn`      | FAIL (wrong value) | FAIL |
//! | `registry_stale_entry_defeats_fail_closed`   | FAIL (wrong value) | FAIL |
//! | `floor_crossing_preserves_fn_pointer_provenance` | pass | FAIL (UB) |
//! | `dyn_fn_self_arg_keeps_a_valid_vtable`       | pass | FAIL (UB) |
//!
//! Run the miri-only canaries with:
//!
//! ```text
//! cargo +nightly miri test --test ub_regressions
//! ```
//!
//! Note that miri aborts a test binary at the *first* UB it sees, so fix the provenance defect
//! (class B) before expecting to observe class C here.

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
#[test]
fn registry_key_collision_calls_wrong_fn() {
    use collide::Fold;

    let add: usize = 7;
    let c_add = move |v: usize| v + add;
    let mul: usize = 3;
    let c_mul = move |v: usize| v * mul + 100_000;

    // The precondition for the collision: identical `type_name` and identical layout.
    assert_eq!(
        std::any::type_name_of_val(&c_add),
        std::any::type_name_of_val(&c_mul),
        "precondition: the two closures must share a type_name to share a registry slot",
    );
    assert_eq!(
        std::mem::size_of_val(&c_add),
        std::mem::size_of_val(&c_mul),
        "precondition: the two closures must share a layout fingerprint",
    );

    // Baseline: with no nested descent, the floor calls the right fn.
    assert_eq!(collide::A.fold(c_add, 30), 37, "baseline (no nested descent)");

    // Now let the body start a nested descent with the colliding closure.
    HOOK.with(|s| {
        *s.borrow_mut() = Some(Box::new(move || {
            let _ = collide::A.fold(c_mul, 12);
        }))
    });

    // `c_add` adds 7 to 0, then 30 frames add 1 each => 37. Observed today: 100030, i.e. `c_mul`'s
    // body (`v * 3 + 100000`) ran in place of `c_add`'s.
    assert_eq!(
        collide::A.fold(c_add, 30),
        37,
        "the floor called the wrong closure body: a nested descent overwrote this frame's \
         registry slot (registration is last-writer-wins, not frame-scoped)",
    );
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

/// Class A, mechanism 3: registry entries are never removed, so an entry left behind by a descent
/// that already hit the documented fail-closed panic silently defeats that backstop on a *later*,
/// unrelated call. The backstop only checks that a key is *present*, never that it is *correct*.
///
/// No nesting and no `unsafe` — just a caught panic (a test harness or a `catch_unwind` request
/// boundary) followed by an ordinary call.
#[test]
fn registry_stale_entry_defeats_fail_closed() {
    use stale::Fold;

    let mul: usize = 3;
    let c_mul = move |v: usize| v * mul + 100_000;
    let add: usize = 7;
    let c_add = move |v: usize| v + add;

    // Step 1: this descent registers a key for `B`, then hits the documented fail-closed panic.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let first = std::panic::catch_unwind(move || stale::B.fold(c_mul, 5));
    // Step 2: an unrelated call whose own descent never registers that key.
    let second = std::panic::catch_unwind(move || stale::A.fold(c_add, 5));
    std::panic::set_hook(prev);

    assert!(
        first.is_err(),
        "precondition: step 1 must hit the documented fail-closed panic",
    );

    // Correct outcomes are either the same fail-closed panic or `12` (`c_add`: 7, plus 5 frames).
    // Observed today: `Ok(100005)` — `c_mul`'s body, resurrected from the stale entry.
    match second {
        Err(_) => {} // fail-closed, as documented
        Ok(v) => assert_eq!(
            v, 12,
            "a stale registry entry from an already-panicked descent was reused, converting a \
             documented fail-closed panic into a silent wrong-fn call",
        ),
    }
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
