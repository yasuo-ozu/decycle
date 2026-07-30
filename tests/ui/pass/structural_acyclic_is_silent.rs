//! `#[decycle(structural)]` on a module with no cyclic participants is a SILENT no-op: the impls are
//! valid Rust on their own, so the engine emits the body unchanged instead of erroring. Here the trait
//! carries no `#[decycle]` and there is no cycle — this must COMPILE. (The ranked engine, by contrast,
//! requires an annotated cycle and rejects the same shape — see `ranked_no_decycle_trait.rs`.)
use decycle::decycle;

#[decycle(structural)]
mod m {
    pub trait Ev {
        fn ev(&self) -> i64;
    }
    pub struct A;
    impl Ev for A {
        fn ev(&self) -> i64 {
            0
        }
    }
}

fn main() {
    use m::Ev;
    assert_eq!(m::A.ev(), 0);
}
