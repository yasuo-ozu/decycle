//! A macro invocation in item position inside a `#[decycle]` module is opaque to the rewrite (decycle
//! can't see what it expands to) — reject it clearly.
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Ev {
        fn ev(&self) -> i64;
    }
    pub struct A;
    impl Ev for A {
        fn ev(&self) -> i64 {
            0
        }
    }

    stray_macro!();
}

fn main() {}
