//! A `#[decycle] use`d trait is the cycle's by the name it is imported as: an impl naming it by its
//! path is the opt-out, left as written, so a cycle through it overflows as without `#[decycle]`.
#[decycle::decycle]
pub trait Loop {
    fn step(&self, n: u32) -> u32;
}

#[decycle::decycle]
mod m {
    #[decycle]
    use super::Loop;

    pub struct A;
    pub struct B;

    impl super::Loop for A
    where
        B: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            n
        }
    }
}

fn main() {}
