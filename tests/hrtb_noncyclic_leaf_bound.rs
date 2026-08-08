//! Ported from cycling:main tests/hrtb_where_bound.rs — a NON-cyclic (leaf) HRTB
//! `where`-predicate carrying a `for<'a>` binder must be preserved verbatim,
//! binder included, on every generated impl, and must be genuinely usable.
//!
//! decycle's `hrtb_reentry.rs` already covers an HRTB bound that IS the cyclic
//! edge (`for<'a> Wrap<&'a A>: Cb`). This is the complementary case: the HRTB
//! bound is a plain non-cyclic leaf obligation (`for<'a> &'a i64: Frob`) sitting
//! alongside the real cyclic bound. Dropping its binder while re-emitting it onto
//! the ranked impls would be an unbound-lifetime E0261 (cycling's original bug),
//! so this doubles as a regression that decycle threads such binders through its
//! rank-rewrite. The bound is exercised at runtime (`(&v).frob()` inside `ca`),
//! and the cycle re-enters well past the floor.

use decycle::decycle;

pub trait Frob {
    fn frob(&self) -> i64;
}

// The explicit binder mirrors the HRTB `for<'a> &'a i64: Frob` under test; clippy's elision
// suggestion would obscure that parallel.
#[allow(clippy::needless_lifetimes)]
impl<'x> Frob for &'x i64 {
    fn frob(&self) -> i64 {
        **self
    }
}

#[decycle]
mod m {
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
        for<'a> &'a i64: crate::Frob,
    {
        fn ca(&self, n: usize) -> usize {
            // Actually uses the HRTB leaf bound: if its binder were dropped from
            // the generated impls this wouldn't compile.
            let v: i64 = 7;
            let _ = (&v).frob();
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

#[test]
fn noncyclic_hrtb_leaf_bound_survives_and_runs_unbounded() {
    use m::Ca;
    assert_eq!(m::A.ca(2000), 2000);
}
