//! A `#[decycle]` module whose trait carries no `#[decycle]` (and there is no `#[decycle] use`) has
//! nothing marking a cycle participant — fail closed instead of silently expanding to a no-op.
use decycle::decycle;

#[decycle]
mod m {
    // NOTE: the trait is NOT annotated `#[decycle]`, and there is no `#[decycle] use`.
    pub trait Ev {
        fn ev(&self) -> i64;
    }
    pub struct A;
    impl Ev for A {
        fn ev(&self) -> i64 {
            0
        }
    }
}

fn main() {}
