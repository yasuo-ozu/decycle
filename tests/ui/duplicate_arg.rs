//! A repeated keyword argument used to silently LAST-WIN (`recurse_level = 5, recurse_level = 2`
//! compiled with the limit at 2). Now the duplicate is rejected, pointing at the second
//! occurrence.
#[decycle::decycle(recurse_level = 5, recurse_level = 2)]
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
            if n == 0 {
                0
            } else {
                B.step(n - 1) + 1
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

fn main() {}
