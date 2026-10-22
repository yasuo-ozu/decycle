//! A call through an impl of the module whose step impl cannot apply must not compile, on every
//! trait solver. `(B, crate::other::Leaf).step(..)` resolves through the pair impl, whose
//! step impl needs `Leaf: LoopRanked<_>`, and `Leaf` (implemented outside the module) has no ranked
//! impl. So the floor of `(X, Y)` was the one impl left, selected by elimination even on the old
//! solver: this program compiled and panicked at run time ("re-entry fn not registered", or "cycle
//! limit reached" with `support_infinite_cycle = false`). The floor's decoy twin keeps a second
//! impl in play, so the call is `E0283`. The fix is the call by path, through the public trait,
//! whose pair impl needs every part's ranked impl: implement `Loop` for `Leaf` in the module too,
//! or move the pair impl out of the module (`tests/floor_by_elimination.rs`). Here, as written,
//! the call by path is `E0277` on `Leaf: LoopRanked<..>`.
use decycle::decycle;

pub mod other {
    pub struct Leaf;
}

#[decycle]
pub mod m {
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
        (B, crate::other::Leaf): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 { 0 } else { (B, crate::other::Leaf).step(n - 1) }
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
