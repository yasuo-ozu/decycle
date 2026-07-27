//! A generic **cross-trait** cycle on one type (`GenA<T>`↔`GenB<T>` on `Boxed<T>`), exercising shapes
//! not covered by the other both-algorithm tests: an associated fn returning `-> Self`, a `&Self`
//! non-receiver argument, owned `self`, a `Self::` constructor in the body, and cross-trait method
//! calls between the two traits. Run under BOTH algorithms.
#![allow(dead_code)]
mod common;

dual_mod! {
    gen {
        #[decycle]
        pub trait GenA<T> {
            fn lift(value: T) -> Self;
            fn pair(&self, other: &Self, tag: usize) -> usize;
        }
        #[decycle]
        pub trait GenB<T> {
            fn scale(self, factor: usize) -> usize;
            fn describe(&self) -> &'static str;
        }

        #[derive(Clone)]
        pub struct Boxed<T> {
            pub value: T,
            pub next: Option<Box<Boxed<T>>>,
        }

        impl<T: Clone> GenA<T> for Boxed<T>
        where
            Boxed<T>: GenB<T>,
        {
            fn lift(value: T) -> Self {
                Self { value, next: None } // `-> Self` construction
            }
            fn pair(&self, other: &Self, tag: usize) -> usize {
                let _ = other.describe(); // cross-trait call GenA -> GenB on a &Self arg
                tag + if self.next.is_some() { 1 } else { 0 }
            }
        }

        impl<T: Clone> GenB<T> for Boxed<T>
        where
            Boxed<T>: GenA<T>,
        {
            fn scale(self, factor: usize) -> usize {
                let _ = Self::lift(self.value.clone()); // `Self::` ctor via the other trait
                factor
            }
            fn describe(&self) -> &'static str {
                "boxed"
            }
        }
    }
}

#[test]
fn generic_cross_trait_self_return_and_arg() {
    on_both!(gen, {
        let a: Boxed<u32> = Boxed::lift(1);
        let b: Boxed<u32> = Boxed::lift(2);
        assert_eq!(a.pair(&b, 5), 5); // a.next is None -> 5
        assert_eq!(b.clone().scale(4), 4);
        assert_eq!(b.describe(), "boxed");
    });
}
