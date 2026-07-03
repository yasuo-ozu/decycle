//! `recurse_level = 0` is rejected: the delegating impl would dispatch straight to the
//! rank floor, which no frame can ever have registered.
#[decycle::decycle(recurse_level = 0)]
mod m {
    #[decycle]
    trait A {
        fn a(&self) -> usize;
    }
    struct X;
    impl A for X
    where
        X: A,
    {
        fn a(&self) -> usize {
            0
        }
    }
}

fn main() {}
