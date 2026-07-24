//! Ported from cycling:main tests/complex_multiroot.rs — a single module holding
//! TWO independent cycles (disjoint SCCs), one of them WIDER than any existing
//! decycle test's cycle (width 4 vs the widest `wide3_*` in unbounded_reentry.rs).
//!
//! cycling detects the SCCs by parsing ADT fields; decycle can't do that, so the
//! same *scenario intent* is expressed with explicit cyclic `where`-bounds:
//!
//!  - Cycle 1 — a four-root ring `A -> B -> C -> D -> A` (all mutually reachable,
//!    one SCC). Every hop adds a constant, so a descent far past the fixed
//!    `recurse_level` floor must accumulate the correct value, not merely forward
//!    a leaf.
//!  - Cycle 2 — an independent two-root SCC `P <-> Q` that references nothing in
//!    cycle 1. Same trait, same module, same shared depth machinery, but a
//!    disjoint set of inductive impls — the two engines must coexist without
//!    interfering.

use decycle::decycle;

#[decycle]
mod twin_cycles {
    #[decycle]
    pub trait Eval {
        fn eval(&self, n: usize) -> usize;
    }

    pub struct A;
    pub struct B;
    pub struct C;
    pub struct D;

    impl Eval for A
    where
        B: Eval,
    {
        fn eval(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.eval(n - 1) + 1
            }
        }
    }
    impl Eval for B
    where
        C: Eval,
    {
        fn eval(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                C.eval(n - 1) + 1
            }
        }
    }
    impl Eval for C
    where
        D: Eval,
    {
        fn eval(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                D.eval(n - 1) + 1
            }
        }
    }
    impl Eval for D
    where
        A: Eval,
    {
        fn eval(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.eval(n - 1) + 1
            }
        }
    }

    // Disjoint second SCC, sharing nothing with the ring above.
    pub struct P;
    pub struct Q;

    impl Eval for P
    where
        Q: Eval,
    {
        fn eval(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                Q.eval(n - 1) + 1
            }
        }
    }
    impl Eval for Q
    where
        P: Eval,
    {
        fn eval(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                P.eval(n - 1) + 1
            }
        }
    }
}

#[test]
fn four_root_ring_is_unbounded_and_accumulates() {
    use twin_cycles::Eval;
    // 400 hops around a width-4 ring, well past the default recurse_level (10):
    // every hop adds 1, so a correctly-connected ring returns n back out.
    assert_eq!(twin_cycles::A.eval(400), 400);
    assert_eq!(twin_cycles::B.eval(401), 401);
    assert_eq!(twin_cycles::C.eval(402), 402);
    assert_eq!(twin_cycles::D.eval(403), 403);
}

#[test]
fn disjoint_two_root_cycle_coexists() {
    use twin_cycles::Eval;
    assert_eq!(twin_cycles::P.eval(500), 500);
    assert_eq!(twin_cycles::Q.eval(501), 501);
    // Interleave the two engines to prove they don't interfere.
    assert_eq!(twin_cycles::A.eval(200), 200);
    assert_eq!(twin_cycles::P.eval(200), 200);
    assert_eq!(twin_cycles::A.eval(201), 201);
}
