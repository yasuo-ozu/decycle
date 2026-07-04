//! D4: a `support_infinite_cycle` (default-on) cycle method returning `impl Trait` can't get
//! an unbounded re-entry fn-pointer alias (`fn(...) -> impl Trait` is E0562, "impl Trait only
//! allowed in function and inherent method return types") — that raw solver error used to leak
//! straight out of decycle's generated code. Now a clean, actionable `abort!` instead.
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
