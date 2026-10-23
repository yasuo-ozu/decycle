//! A glob `#[decycle] use path::*;` names no trait, so it can't say which are the cycle
//! participants: a `#[decycle(structural)]` module rejects it as the ranked engine does
//! (`ranked_glob_use.rs`). It used to route nothing, silently.
use decycle::decycle;

mod ext {
    pub trait Ev {
        fn ev(&self) -> i64;
    }
}

#[decycle(structural)]
mod m {
    #[decycle]
    use crate::ext::*;

    pub struct A;
    impl crate::ext::Ev for A {
        fn ev(&self) -> i64 {
            0
        }
    }
}

fn main() {}
