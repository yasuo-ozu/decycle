//! Ported from cycling:main tests/e3_trait_args.rs — a participating trait
//! carrying a CONST own-argument (`trait Frob<const N: usize>`), recursing across
//! a cycle past the floor.
//!
//! decycle's CHANGELOG (0.4.0) lists "Trait-level const generic parameters on a
//! #[decycle] trait are now supported (previously E0747 in generated code)", but
//! its test suite exercises const generics only on the SELF TYPE
//! (`more_cycles.rs`'s `ArrayHolder<const N>`), never as a trait own-arg. This
//! locks that fix with a runtime regression: the const arg rides every ranked
//! impl header, and the cycle re-enters unbounded.

use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Frob<const N: usize> {
        fn frob(&self, n: usize) -> usize;
    }

    pub struct A;
    pub struct B;

    impl<const N: usize> Frob<N> for A
    where
        B: Frob<N>,
    {
        fn frob(&self, n: usize) -> usize {
            if n == 0 {
                N
            } else {
                B.frob(n - 1) + 1
            }
        }
    }

    impl<const N: usize> Frob<N> for B
    where
        A: Frob<N>,
    {
        fn frob(&self, n: usize) -> usize {
            if n == 0 {
                N
            } else {
                A.frob(n - 1) + 1
            }
        }
    }
}

#[test]
fn trait_const_own_arg_cycle_is_unbounded() {
    use m::Frob;
    // Each hop adds 1, floor returns N, so the total is n + N, unbounded. N is a
    // trait own-arg (not inferable from the args) so it is named via the qself.
    assert_eq!(<m::A as Frob<3>>::frob(&m::A, 2000), 2000 + 3);
    assert_eq!(<m::A as Frob<5>>::frob(&m::A, 2000), 2000 + 5);
}
