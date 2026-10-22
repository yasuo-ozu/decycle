//! A wrapped bound is replaced by the obligations of its parts: `(B, crate::m::C): Loop` is the edge
//! `B: Loop` and the premise `crate::m::C: Loop`, as written. A cycle type named by a path is the
//! opt-out, so the cycle through it overflows at compile time. (The part used to be dropped, and the
//! body's call through it panicked at run time: "re-entry fn not registered".)
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;
    pub struct C;

    impl Loop for A
    where
        (B, crate::m::C): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            B.step(n) + C.step(n)
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            A.step(n)
        }
    }

    impl Loop for C
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            A.step(n)
        }
    }
}

fn main() {}
