//! A `module` path that names no module: one error at the argument, not one per qualified name
//! (each `crate::nowhere::r::X` fails on the same `nowhere` token of the argument).
#[decycle::decycle(module = crate::nowhere::r)]
pub mod r {
    #[decycle]
    pub trait Loop {
        fn step(&self) -> u32;
    }

    pub struct Leaf;
    pub struct Holder(pub Leaf);
    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self) -> u32 {
            1
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self) -> u32 {
            2
        }
    }
}

#[decycle::decycle(structural, module = crate::nowhere::s)]
pub mod s {
    #[decycle]
    pub trait Loop {
        fn step(&self) -> u32;
    }

    pub struct Leaf;
    pub struct Holder(pub Leaf);
    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self) -> u32 {
            1
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self) -> u32 {
            2
        }
    }
}

fn main() {}
