//! Ported from cycling:main tests/d2_construction.rs — a *construction*-style
//! participating trait method with NO `self` receiver, driven UNBOUNDED past the
//! floor through associated-fn call paths (`A::build(..)` / `<A as Build>::build(..)`).
//!
//! decycle's `advanced_cycles.rs` exercises receiver-less methods (`build`,
//! `create`, `make`) but only SHALLOWLY (called once, never past the floor). The
//! interesting case is a receiver-less method whose re-entry fn has no `self`
//! first parameter yet must still register-before-descend and re-enter at full
//! height. Here `build` recurses across a two-type cycle with no receiver, adding
//! a constant each hop, so a correct unbounded descent returns `n`.

use decycle::decycle;

#[decycle]
mod builder {
    #[decycle]
    pub trait Build {
        fn build(n: usize) -> usize;
    }

    pub struct A;
    pub struct B;

    impl Build for A
    where
        B: Build,
    {
        fn build(n: usize) -> usize {
            if n == 0 {
                0
            } else {
                // Cross-edge via an associated-fn path, no receiver.
                <B as Build>::build(n - 1) + 1
            }
        }
    }

    impl Build for B
    where
        A: Build,
    {
        fn build(n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A::build(n - 1) + 1
            }
        }
    }
}

#[test]
fn receiverless_construction_reentry_is_unbounded() {
    use builder::Build;
    assert_eq!(<builder::A as Build>::build(2000), 2000);
    assert_eq!(builder::B::build(2001), 2001);
}
