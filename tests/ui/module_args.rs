//! `allowed_paths` was an option of the trait-side `#[decycle]` (the type-leak allowlist), which was
//! removed in 0.6 together with `marker` and `mock`. Written on a `#[decycle]` MODULE it is rejected
//! with the same message; the module is still processed first, so this stays a single error.
#[decycle::decycle(allowed_paths = [::core::primitive::u32])]
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
