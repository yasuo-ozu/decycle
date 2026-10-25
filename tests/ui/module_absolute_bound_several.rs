//! Every opt-out bound on a cycle is reported, each at its own bound with its own chain: a cycle
//! whose two edges are both spelled by a path, and an impl bounding itself by its path. An opt-out
//! whose impl does not lead back (`crate::m::Leaf`), and one that reaches a cycle nothing leads back
//! from (`D`'s), are not.
#[decycle::decycle(module = crate::m)]
pub mod m {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;
    pub struct C;
    pub struct D;
    pub struct Leaf;

    impl Loop for A
    where
        crate::m::B: Loop,
        crate::m::Leaf: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }

    impl Loop for B
    where
        crate::m::A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }

    impl Loop for C
    where
        crate::m::C: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }

    impl Loop for D
    where
        crate::m::A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }

    impl Loop for Leaf {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }
}

fn main() {}
