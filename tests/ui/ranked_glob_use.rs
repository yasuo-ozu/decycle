//! A glob `#[decycle] use path::*;` can't name which traits are the cycle participants — reject it
//! (an explicit `use path::Trait;` is required).
use decycle::decycle;

mod ext {
    pub trait Ev {
        fn ev(&self) -> i64;
    }
}

#[decycle]
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
