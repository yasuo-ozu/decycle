//! The markers of a cyclic module are checked where the engine takes them, item by item in source
//! order. A glob `#[decycle] use` before a trait whose marker carries arguments is the first error:
//! `glob is not supported in #[decycle] use`. The value-generic growth pre-pass used to check the
//! markers of the module's traits first, so the trait's `takes no arguments` was reported instead.
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

    #[decycle(recurse_level = 3)]
    pub trait Tr {
        fn f(&self) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Tr for A
    where
        B: Tr,
    {
        fn f(&self) -> u32 {
            B.f() + 1
        }
    }

    impl Tr for B
    where
        A: Tr,
    {
        fn f(&self) -> u32 {
            0
        }
    }
}

fn main() {}
