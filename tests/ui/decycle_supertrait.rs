//! L-M3: a #[decycle] trait cannot be a supertrait of another #[decycle] trait in the
//! same batch (the ranked-trait definitions would become mutually referential in a way
//! the rank-rewriting scheme can't discharge — E0283 downstream).
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Base {
        fn base(&self) -> u32;
    }

    #[decycle]
    pub trait Derived: Base {
        fn derived(&self) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Base for A
    where
        B: Base,
    {
        fn base(&self) -> u32 {
            1
        }
    }
    impl Base for B
    where
        A: Base,
    {
        fn base(&self) -> u32 {
            2
        }
    }
    impl Derived for A
    where
        B: Derived,
    {
        fn derived(&self) -> u32 {
            1
        }
    }
    impl Derived for B
    where
        A: Derived,
    {
        fn derived(&self) -> u32 {
            2
        }
    }
}

fn main() {}
