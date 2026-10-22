//! With `module = path` too, only a bare name is the module's: a bound naming a cycle type by a path
//! (`crate::m::B: Loop`) is the opt-out, left as written, so the cycle through it overflows. The
//! module path only qualifies the bare names of the items emitted as written, after the cycle is
//! solved.
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
