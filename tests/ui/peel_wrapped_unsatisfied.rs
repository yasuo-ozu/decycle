//! A wrapped bound is replaced by the obligations of its parts, and a part that contains no cycle
//! member stays a premise on the original trait: `(B, u8): Loop` needs `u8: Loop`, which nothing
//! implements, as plain Rust would report it. (The part used to be dropped, which accepted the
//! bound silently.)
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        (B, u8): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                B.step(n - 1)
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
                A.step(n - 1)
            }
        }
    }
}

fn main() {}
