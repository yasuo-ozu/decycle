//! `module = path` must end in the name of the module it is on: the engines re-spell the module's
//! types with it, so a path to another module would silently mean other types.
#[decycle::decycle(module = crate::other)]
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
