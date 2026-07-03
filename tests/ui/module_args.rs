//! F5: `allowed_paths` configures the `type-leak` allowlist for a trait defined OUTSIDE a
//! `#[decycle]` module (a TRAIT-only argument, same family as `marker`); writing it on a
//! `#[decycle]` MODULE used to be silently ignored instead of rejected like its siblings
//! `marker`/`alter_macro_name` — now a clean `abort!`, matching them.
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
