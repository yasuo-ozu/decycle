//! A participating trait carrying its OWN generic type parameter (`trait Cast<T>`), a single generic
//! impl driven at TWO concrete instantiations (`Cast<i64>` / `Cast<i32>`) on the same deep value. Run
//! under BOTH algorithms — the structural side keeps the bare `B: Cast<T>` bound on the local body
//! impl so `T` in `B.cast(n-1)` (and the floor's `size_of::<T>()`) stays inferable.
mod common;

dual_mod! {
    m {
        #[decycle]
        pub trait Cast<T> {
            fn cast(&self, n: usize) -> usize;
        }

        pub struct A;
        pub struct B;

        impl<T> Cast<T> for A
        where
            B: Cast<T>,
        {
            fn cast(&self, n: usize) -> usize {
                if n == 0 { core::mem::size_of::<T>() } else { B.cast(n - 1) + 1 }
            }
        }

        impl<T> Cast<T> for B
        where
            A: Cast<T>,
        {
            fn cast(&self, n: usize) -> usize {
                if n == 0 { core::mem::size_of::<T>() } else { A.cast(n - 1) + 1 }
            }
        }
    }
}

#[test]
fn generic_trait_arg_instantiations_key_apart_past_floor() {
    on_both!(m, {
        // i64 floor = size_of::<i64>() = 8; i32 floor = 4; each hop adds 1.
        assert_eq!(<A as Cast<i64>>::cast(&A, 2000), 2000 + 8);
        assert_eq!(<A as Cast<i32>>::cast(&A, 2000), 2000 + 4);
        // Interleave to prove the two monomorphizations don't clobber each other.
        for _ in 0..50 {
            assert_eq!(<A as Cast<i64>>::cast(&A, 40), 40 + 8);
            assert_eq!(<A as Cast<i32>>::cast(&A, 40), 40 + 4);
        }
    });
}
