//! A `#[decycle]` on an item of a plain module nested in a `#[decycle(structural)]` module is
//! rejected as in a ranked one: neither engine reads a nested module's items. The trait used to be
//! expanded on its own, as a trait outside any `#[decycle]` module.
use decycle::decycle;

#[decycle(structural)]
mod outer {
    pub mod inner {
        use decycle::decycle;

        #[decycle]
        pub trait Ev {
            fn ev(&self) -> i64;
        }
    }

    pub struct A;
    impl inner::Ev for A {
        fn ev(&self) -> i64 {
            0
        }
    }
}

fn main() {}
