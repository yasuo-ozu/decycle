//! L-m2: an unsupported item (here, a macro invocation) inside an impl of a #[decycle]
//! trait used to reach an `unimplemented!()`/panic in the macro itself; it's a clean
//! `abort!` now.
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                B.step(n - 1) + 1
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
        some_unsupported_macro!();
    }
}

fn main() {}
