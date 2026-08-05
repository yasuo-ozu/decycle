//! `#[decycle]` twice on one module used to yield a nonsense diagnostic — the second expansion
//! tripped over the first's generated paths ("paths with multiple super segments are not
//! supported…" with no `super` in sight). The outer invocation now detects the still-pending
//! inner attribute and says what actually happened.
#[decycle::decycle]
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
