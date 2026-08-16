//! The same rejection reaches the OTHER marker sites for free, because it lives on the predicate
//! every engine matches with: here a `#[decycle] use` (not a trait) inside a `#[decycle(structural)]`
//! module (not the ranked one). `marker` is a trait-only argument and was silently dropped.
use decycle::decycle;

mod ext {
    pub trait Ev {
        fn ev(&self) -> i64;
    }
}

#[decycle(structural)]
mod m {
    #[decycle(marker = ::nope::Marker)]
    use crate::ext::Ev;

    pub struct A;
    pub struct B;

    impl Ev for A
    where
        B: Ev,
    {
        fn ev(&self) -> i64 {
            B.ev() + 1
        }
    }

    impl Ev for B
    where
        A: Ev,
    {
        fn ev(&self) -> i64 {
            0
        }
    }
}

fn main() {}
