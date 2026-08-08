//! Return-position `impl Trait` (RPITIT) in a `#[decycle]` (ranked) trait method is a HARD ERROR:
//! the ranked re-entry fn-pointer type `fn(..) -> impl Trait` is not nameable (E0562), so decycle
//! rejects it up-front rather than leaking a raw solver error. The help points at an associated type.
//! (Default `support_infinite_cycle`; the bounded-mode counterpart is rejected identically — see
//! `ranked_impl_trait_return_bounded.rs`.)
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> impl Iterator<Item = u8>;
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
    {
        fn ca(&self, n: usize) -> impl Iterator<Item = u8> {
            let _ = B.cb(n);
            ::core::iter::once(0u8)
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            n
        }
    }
}

fn main() {}
