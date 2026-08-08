//! Ported from cycling:main tests/mp_qualified_trait_call.rs — a recursive call
//! spelled through a FULLY-QUALIFIED, crate-rooted qself path
//! (`<A as crate::Loop>::step(..)`, 3 path segments) must still be recognized and
//! routed through the ranked engine, exactly like the bare `.step()` /
//! `Loop::step(..)` spellings.
//!
//! decycle's `legacy_fixes.rs` covers the `self::Loop` qself form (L-M5a) and the
//! two-segment `Loop::step(..)` value path (L-M5b). This adds the three-segment
//! `crate::`-rooted qself variant. The trait is declared at the file's own top
//! level so `crate::Loop` is a valid path from inside the generated shadow module;
//! it is pulled into the module with `#[decycle] use crate::Loop;`.

use decycle::decycle;

#[decycle]
pub trait Loop {
    fn step(&self, n: usize) -> usize;
}

#[decycle]
mod m {
    #[decycle]
    use crate::Loop;

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                <B as crate::Loop>::step(&B, n - 1) + 1
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                <A as crate::Loop>::step(&A, n - 1) + 1
            }
        }
    }
}

#[test]
fn crate_qualified_qself_call_is_rewritten_and_unbounded() {
    use m::A;
    assert_eq!(A.step(2000), 2000);
}
