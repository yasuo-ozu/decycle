//! Return-position `impl Trait` in a `#[decycle(structural)]` trait method is a HARD ERROR — an
//! opaque return type the layout cast can't reinterpret (the same soundness class as `async fn`).
//! Rejected up-front, with the help pointing at an associated type.
use decycle::decycle;

#[decycle(structural)]
mod m {
    #[decycle]
    pub trait Ev {
        fn ev(&self) -> impl std::fmt::Debug;
    }
    pub struct A;
    impl Ev for A
    where
        A: Ev,
    {
        fn ev(&self) -> impl std::fmt::Debug {
            0i64
        }
    }
}

fn main() {}
