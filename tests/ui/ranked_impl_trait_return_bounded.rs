//! Return-position `impl Trait` is rejected in the ranked engine REGARDLESS of mode — including
//! `support_infinite_cycle = false`. (It was previously allowed in bounded mode when the diverging
//! rank-floor body let the hidden type infer `()`; that fragile special case is gone in favor of a
//! uniform hard error that steers you to an associated type.)
use decycle::decycle;

#[decycle(support_infinite_cycle = false)]
mod m {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> impl std::fmt::Debug;
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
        fn ca(&self, n: usize) -> impl std::fmt::Debug {
            B.cb(n)
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

fn main() {
    use m::Ca;
    let _ = m::A.ca(3);
}
