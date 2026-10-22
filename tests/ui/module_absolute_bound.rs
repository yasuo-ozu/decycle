//! With `module = path` too, only a bare name is the module's: a bound naming a cycle type by a path
//! (`crate::m::B: Loop`) is the opt-out, left as written, so the cycle through it overflows. The
//! module path only qualifies the bare names of the items emitted as written, after the cycle is
//! solved. It also proves that `crate::m::B` is this module's `B`, so the overflow is reported at the
//! bound, in the module's terms, instead of as rustc's `E0275` on the engine's `LoopRanked` trait.
#[decycle::decycle(module = crate::m)]
mod m {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        crate::m::B: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }
}

fn main() {}
