//! `#[decycle]` on a trait alias is rejected by a `#[decycle(structural)]` module as by the ranked
//! engine (`trait_alias.rs`): an alias has no body to take part in a cycle. The marker used to be
//! left on the alias.
#[decycle::decycle(structural)]
mod m {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }

    #[decycle]
    trait CaAlias = Ca;

    pub struct A;

    impl Ca for A {
        fn ca(&self, n: usize) -> usize {
            n
        }
    }
}

fn main() {}
