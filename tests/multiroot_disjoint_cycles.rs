//! A single module holding TWO independent cycles (disjoint SCCs) — a four-root ring
//! `A -> B -> C -> D -> A` and an independent two-root `P <-> Q`. Run under BOTH algorithms.
mod common;

dual_mod! {
    twin_cycles {
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
                if n == 0 { 0 } else { B.eval(n - 1) + 1 }
            }
        }
        impl Eval for B
        where
            C: Eval,
        {
            fn eval(&self, n: usize) -> usize {
                if n == 0 { 0 } else { C.eval(n - 1) + 1 }
            }
        }
        impl Eval for C
        where
            D: Eval,
        {
            fn eval(&self, n: usize) -> usize {
                if n == 0 { 0 } else { D.eval(n - 1) + 1 }
            }
        }
        impl Eval for D
        where
            A: Eval,
        {
            fn eval(&self, n: usize) -> usize {
                if n == 0 { 0 } else { A.eval(n - 1) + 1 }
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
                if n == 0 { 0 } else { Q.eval(n - 1) + 1 }
            }
        }
        impl Eval for Q
        where
            P: Eval,
        {
            fn eval(&self, n: usize) -> usize {
                if n == 0 { 0 } else { P.eval(n - 1) + 1 }
            }
        }
    }
}

#[test]
fn four_root_ring_is_unbounded_and_accumulates() {
    on_both!(twin_cycles, {
        // 400 hops around a width-4 ring, well past the default recurse_level (ranked).
        assert_eq!(A.eval(400), 400);
        assert_eq!(B.eval(401), 401);
        assert_eq!(C.eval(402), 402);
        assert_eq!(D.eval(403), 403);
    });
}

#[test]
fn disjoint_two_root_cycle_coexists() {
    on_both!(twin_cycles, {
        assert_eq!(P.eval(500), 500);
        assert_eq!(Q.eval(501), 501);
        // Interleave the two SCCs to prove they don't interfere.
        assert_eq!(A.eval(200), 200);
        assert_eq!(P.eval(200), 200);
        assert_eq!(A.eval(201), 201);
    });
}
