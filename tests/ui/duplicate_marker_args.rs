//! Every `#[decycle]` marker of an item is checked, a second one too: an argument list on it is
//! rejected by the ranked engine, which used to check only the first.
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    #[decycle(recurse_level = 3)]
    pub trait Tr {
        fn f(&self) -> u32;
    }

    pub struct A;

    impl Tr for A {
        fn f(&self) -> u32 {
            0
        }
    }
}

fn main() {}
