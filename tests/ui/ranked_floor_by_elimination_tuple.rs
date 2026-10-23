//! A call through an impl of the module whose step impl cannot apply must not compile, on every
//! trait solver. `(B, Foreign).step(..)` resolves through the pair impl, whose step impl needs
//! `Leaf: LoopRanked<_>`. A part outside the cycle normally gets a ranked impl forwarding to `Loop`
//! (so the same pair with `crate::other::Leaf` spelled by its path compiles and runs,
//! `tests/wrapper_bridge.rs`), but not one the module names through a `use .. as` rename: a renamed
//! name may be a cycle member under another name, whose own ranked impls such a forwarding impl
//! would conflict with. So the floor of `(X, Y)` is what is left, and selected by elimination this
//! program used to compile and panic at run time ("re-entry fn not registered", or "cycle limit
//! reached" with `support_infinite_cycle = false`). The floor's decoy twin keeps a second impl in
//! play, so the call is `E0283`. The fix is to name the part by its path or by its own name.
use decycle::decycle;

pub mod other {
    pub struct Leaf;
}

#[decycle]
pub mod m {
    use crate::other::Leaf as Foreign;

    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }
    pub struct A;
    pub struct B;
    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n)
        }
    }
    impl Loop for A
    where
        (B, Foreign): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 { 0 } else { (B, Foreign).step(n - 1) }
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

impl m::Loop for other::Leaf {
    fn step(&self, _n: u32) -> u32 {
        100
    }
}

fn main() {
    use m::Loop;
    m::A.step(5);
}
