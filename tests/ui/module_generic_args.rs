//! A module path takes no generic arguments.
#[decycle::decycle(module = crate::m<u8>)]
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
            if n == 0 { 0 } else { B.step(n - 1) + 1 }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 { 0 } else { A.step(n - 1) + 1 }
        }
    }
}

fn main() {}
