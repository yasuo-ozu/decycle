//! A WRAPPED cyclic bound (`Box<Stmt>: Tr`) is **peeled** to the cycle member it contains
//! (`Stmt: Tr`) before ranking, so this compiles.
//!
//! It used to be a hard error: `Box<Stmt>: TrRanked<Rank>` has no floor impl for the container, so
//! rustc emitted a raft of raw overflow errors and ranked rejected the shape up-front instead. The
//! bound carries all the information needed to fix it — the cycle member is right there in the type
//! argument — so `ranked::peel` now rewrites it rather than refusing it. See that module for why the
//! two forms are the same obligation as far as the cycle is concerned.
//!
//! NOTE the deliberate asymmetry with the structural engine: `wrapped_bound_not_forwarded.rs` still
//! requires the container to genuinely forward the trait (`for<X: Tr> Box<X>: Tr`), because
//! structural *strips* the bound and needs the forwarding impl to stay sound. Ranked does not strip
//! it, it re-targets it, so no blanket impl is involved.
use decycle::decycle;

#[decycle]
mod ast {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> i64;
    }
    pub enum Stmt {
        Leaf(i64),
        Node(Box<Stmt>),
    }
    impl Tr for Stmt
    where
        Box<Stmt>: Tr,
    {
        fn f(&self) -> i64 {
            match self {
                Stmt::Leaf(n) => *n,
                Stmt::Node(b) => b.f(),
            }
        }
    }
}

fn main() {}
