//! In a module that is itself a submodule, `super::m::C` is a path to the module's own `C` too:
//! `super` is resolved against the module path (`crate::outer::m`), so the overflow through it is
//! reported like the one through `crate::outer::m::C`.
pub mod outer {
    #[decycle::decycle(module = crate::outer::m)]
    pub mod m {
        #[decycle]
        pub trait Loop {
            fn step(&self, n: u32) -> u32;
        }

        pub struct A;
        pub struct C;

        impl Loop for A
        where
            super::m::C: Loop,
        {
            fn step(&self, n: u32) -> u32 {
                n
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
}

fn main() {}
