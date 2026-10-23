//! A leading `::` is a path, so an impl of `::Ca` names the item `::Ca` itself, the opt-out,
//! left as written as in the structural engine (`::Ca` is a crate in edition 2018 on, as rustc
//! says). The ranked engine used to read it as the bare `Ca`, and rewrite it into the cycle.
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }

    pub struct A;
    pub struct B;

    impl Ca for A
    where
        B: Ca,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 { 0 } else { B.ca(n - 1) + 1 }
        }
    }

    impl ::Ca for B
    where
        A: Ca,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 { 0 } else { A.ca(n - 1) + 1 }
        }
    }
}

fn main() {}
