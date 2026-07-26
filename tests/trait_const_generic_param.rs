//! A participating trait carrying a CONST own-argument (`trait Frob<const N: usize>`), recursing
//! across a cycle. Run under BOTH algorithms. The structural side keeps the bare cyclic bound
//! `B: Frob<N>` on the local body impl so `N` in `B.frob(n-1)` stays inferable.
mod common;

dual_mod! {
    m {
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
                if n == 0 { N } else { B.frob(n - 1) + 1 }
            }
        }

        impl<const N: usize> Frob<N> for B
        where
            A: Frob<N>,
        {
            fn frob(&self, n: usize) -> usize {
                if n == 0 { N } else { A.frob(n - 1) + 1 }
            }
        }
    }
}

#[test]
fn trait_const_own_arg_cycle_is_unbounded() {
    on_both!(m, {
        // Each hop adds 1, floor returns N, so the total is n + N. N is a trait own-arg (not
        // inferable from the args) so it is named via the qself.
        assert_eq!(<A as Frob<3>>::frob(&A, 2000), 2000 + 3);
        assert_eq!(<A as Frob<5>>::frob(&A, 2000), 2000 + 5);
    });
}
