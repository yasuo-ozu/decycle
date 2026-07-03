//! L-m4: `check_assoc_type_self` used to only catch a bare `type Assoc = Self;` — a
//! `Self` occurring anywhere in the assigned type (`Box<Self>`, `(Self,)`, …) recreates
//! exactly the same infinite recursive definition and is now caught too.
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Loop {
        type Assoc: Loop;
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        type Assoc = Box<Self>;
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
        type Assoc = B;
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
