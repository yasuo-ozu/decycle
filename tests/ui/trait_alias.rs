//! A trait alias has no body to carry through the macro ping-pong (there's no `ItemTrait`
//! to embed), so `#[decycle]` on one used to silently produce a bogus working-list entry
//! with nothing behind it. Rejected with a clean `abort!` instead.
#[decycle::decycle]
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
