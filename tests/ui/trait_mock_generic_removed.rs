//! The same for a removed option followed by another argument: the value `crate::real::Size<u8, u16>`
//! ends at the comma that follows its `>`, and the module is still rejected for `mock` alone.
#[decycle::decycle(mock = crate::real::Size<u8, u16>, recurse_level = 3)]
mod m {
    #[decycle]
    pub trait T {
        fn f(&self) -> usize;
    }
    pub struct A;
    impl T for A {
        fn f(&self) -> usize {
            0
        }
    }
}

fn main() {}
