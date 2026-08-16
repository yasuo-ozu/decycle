//! An INNER `#[decycle]` marker takes no arguments — and every argument here used to be discarded
//! in silence, including ones that are hard errors on the outer attribute. The worst of them is
//! `structural`: written on the trait instead of on the `mod`, the user asked for the structural
//! engine, got no diagnostic at all, and was quietly handed the ranked one.
//!
//! (`recurse_level = 0`, an unknown keyword and a `marker` path are in the same list to show that
//! none of them is read either; each is a clean abort at the top level.)
use decycle::decycle;

#[decycle]
mod m {
    #[decycle(structural, recurse_level = 0, marker = Nonexistent, bogus_keyword)]
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
