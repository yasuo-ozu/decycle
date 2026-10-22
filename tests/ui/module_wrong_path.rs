//! A `module` path that is well-formed but not this module's: `crate::a::r` names another module,
//! which declares a `Leaf` too, so the qualified field `crate::a::r::Leaf` would quietly be the other
//! module's. The path is checked against the module, with one error at the argument.
pub mod a {
    pub mod r {
        pub struct Leaf;
    }
    pub mod s {
        pub struct Leaf;
    }
}

#[decycle::decycle(module = crate::a::r)]
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

#[decycle::decycle(structural, module = crate::a::s)]
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
