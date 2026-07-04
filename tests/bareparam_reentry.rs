//! Acceptance test for decycle fix **C4** (bare-param cyclic-bound registration,
//! `docs/decycle-integration/impl-decycle-registration-and-bridge.md`).
//!
//! `impl<T: Cb> Ca for Wrap<T>` (and its mirror `impl<T: Ca> Cb for Wrap<T>`) is a bare-param
//! cyclic bound: `impl_has_bare_param_cyclic_bound` returns true for it, so `rule1_registration_ok`
//! skips it, and `cyclic_where_bounds` skips a bare-param *target* too — meaning, PRE-C4, a
//! `Wrap<Concrete>`'s own `Self: Ca`/`Self: Cb` obligation is registered by NO frame at all. Its
//! bare (rank-0) floor is only reached as a cross-edge from ANOTHER type's induction step
//! exhausting the shared `recurse_level` budget in a single hop — exactly what happens here: the
//! OUTER `Wrap<Wrap<Leaf>>`'s single inductive step (at `recurse_level = 1`) needs
//! `Wrap<Leaf>: CbRanked<()>`, i.e. `Wrap<Leaf>`'s own bare floor. Before C4, entering
//! `Wrap<Leaf>` directly as a root (through the original `Cb` trait, i.e. through its Final
//! delegating impl) registered nothing for it, so this floor's `lookup` always failed —
//! documented "re-entry fn not registered" panic, confirmed below by reverting the fix.
//!
//! C4 re-homes this registration to the Final delegating impl (outside the rank rewrite, where
//! `Self: Cb`/`Self: Ca` is an assumed environment bound), so entering `Wrap<Leaf>` directly
//! once (`priming`, below) is enough for the OUTER wrapper's floor lookup to keep succeeding no
//! matter how deep the value-level recursion `n` goes: `Re_Cb_cb::<Wrap<Leaf>>` re-invokes
//! `Wrap<Leaf>`'s Final impl on every subsequent re-entry, which re-registers on the way down.

use decycle::decycle;

#[decycle(recurse_level = 1)]
mod bareparam {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct Wrap<T>(pub T);
    pub struct Leaf;

    // The bare-param wrapper under test (mirrors syan's container shim
    // `impl<T: __ParseDyn> __ParseDyn for Vec<T>`): a CONCRETE in-module `Wrap`, generic over
    // any `T: Cb` (resp. `T: Ca`). Its own `Self: Ca`/`Self: Cb` obligation is registered by NO
    // frame pre-C4.
    impl<T: Cb> Ca for Wrap<T> {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                self.0.cb(n - 1) + 1
            }
        }
    }
    impl<T: Ca> Cb for Wrap<T> {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                self.0.ca(n - 1) + 1
            }
        }
    }

    // A non-cyclic base case: propagates `n` through unchanged, so a correct unbounded descent
    // is verifiable by exact value, not just "didn't panic".
    impl Ca for Leaf {
        fn ca(&self, n: usize) -> usize {
            n
        }
    }
    impl Cb for Leaf {
        fn cb(&self, n: usize) -> usize {
            n
        }
    }
}

#[test]
fn bareparam_wrap_reentry_is_unbounded_once_primed() {
    use bareparam::{Ca, Cb};

    // Prime the INNER `Wrap<Leaf>`'s own `Self: Cb` obligation by entering it directly through
    // the original `Cb` trait (i.e. through its Final delegating impl) -- this fires exactly the
    // C4 registration under test. Also prime `Leaf` itself (an ORDINARY, non-bare-param rule-1
    // registration, unrelated to C4): `Leaf`'s own floor is reached at rank 0 too, since
    // `Wrap<Leaf>` consumes the whole `recurse_level = 1` budget in a single inductive step.
    assert_eq!(bareparam::Wrap(bareparam::Leaf).cb(0), 0);
    assert_eq!(bareparam::Leaf.ca(0), 0);
    assert_eq!(bareparam::Leaf.cb(0), 0);

    // Drive the OUTER `Wrap<Wrap<Leaf>>` well past `recurse_level = 1`. Its single inductive step
    // needs `Wrap<Leaf>: CbRanked<()>` -- `Wrap<Leaf>`'s OWN bare floor -- which is now
    // registered (by the priming call above, via C4) and re-registers on every re-entry, so this
    // runs UNBOUNDED instead of hitting the documented "re-entry fn not registered" panic.
    // (`Leaf`'s base case propagates `n` unchanged, so the wrapper is the identity function.)
    let v = bareparam::Wrap(bareparam::Wrap(bareparam::Leaf));
    for n in [0usize, 1, 2, 50, 2000] {
        assert_eq!(v.ca(n), n, "unbounded descent must still compute correctly at n={n}");
    }
}
