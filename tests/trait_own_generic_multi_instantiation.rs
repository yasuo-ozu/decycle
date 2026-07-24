//! Ported from cycling:main tests/d1_trait_generics.rs — a participating trait
//! carrying its OWN generic type parameter (`trait Cast<T>`), where a single
//! generic impl is driven at TWO concrete instantiations on the same deep value,
//! which must key apart in the runtime re-entry registry.
//!
//! decycle's `complex_cycles.rs` / `more_cycles.rs` have generic traits, but none
//! recurses ACROSS the cycle through the trait's own arg past the floor. Here the
//! `Cast<T>` cycle re-enters unbounded, and the floor returns a `T`-dependent
//! value (`size_of::<T>()`), so a registry that failed to separate `A@Cast<i64>`
//! from `A@Cast<i32>` (the D1 vtable-key-apart concern) would return the wrong
//! width. Both instantiations live in the same binary and are interleaved.

use decycle::decycle;

#[decycle]
mod m {
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
            if n == 0 {
                core::mem::size_of::<T>()
            } else {
                B.cast(n - 1) + 1
            }
        }
    }

    impl<T> Cast<T> for B
    where
        A: Cast<T>,
    {
        fn cast(&self, n: usize) -> usize {
            if n == 0 {
                core::mem::size_of::<T>()
            } else {
                A.cast(n - 1) + 1
            }
        }
    }
}

#[test]
fn generic_trait_arg_instantiations_key_apart_past_floor() {
    use m::Cast;
    // Even depth ends back on A's floor for the matching T; each hop adds 1.
    // i64 floor = size_of::<i64>() = 8; i32 floor = 4.
    assert_eq!(<m::A as Cast<i64>>::cast(&m::A, 2000), 2000 + 8);
    assert_eq!(<m::A as Cast<i32>>::cast(&m::A, 2000), 2000 + 4);
    // Interleave to prove the two monomorphized engines don't clobber each other.
    for _ in 0..50 {
        assert_eq!(<m::A as Cast<i64>>::cast(&m::A, 40), 40 + 8);
        assert_eq!(<m::A as Cast<i32>>::cast(&m::A, 40), 40 + 4);
    }
}
