//! A wrapper whose impl is OUTSIDE the module (`impl<X: Loop, Y: Loop> Loop for (X, Y)` next to it)
//! has no ranked impl, so a call through it by a method in a re-emitted body finds no method: the
//! bare name of a `#[decycle]` trait reaches only its cycle members there. It is reached by the
//! trait's path (`<(B, Leaf) as crate::m::Loop>::step(..)`, `tests/wrapper_bridge.rs`). (A wrapper the
//! module implements is kept ranked, and a call through it works.)
pub mod other {
    pub struct Leaf;
    impl crate::m::Loop for Leaf {
        fn step(&self, _n: u32) -> u32 {
            0
        }
    }
}

impl<X: m::Loop, Y: m::Loop> m::Loop for (X, Y) {
    fn step(&self, n: u32) -> u32 {
        self.0.step(n) + self.1.step(n)
    }
}

#[decycle::decycle]
pub mod m {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        (B, crate::other::Leaf): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                (B, crate::other::Leaf).step(n - 1) + 1
            }
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
}

fn main() {}
