//! A cyclic-trait bound whose target is a FOREIGN type sharing its last path segment with a local
//! cycle head. `crate::other::Stmt` is a different item from the module's own `Stmt`, so it must
//! not be peeled as a cycle member (peeling on the last segment alone produced a 14-error per-rank
//! wall of `other::Stmt: TrRanked<((((..` at the attribute span).
//!
//! Such a bound is now kept as an ordinary premise on the ORIGINAL trait (M2 — see
//! `requalify_foreign_premises` in `finalize.rs`): with a matching `impl m::Tr for
//! Vec<other::Stmt>` the program compiles and runs (`tests/ranked_foreign_premise.rs`). This
//! snapshot pins what happens WITHOUT that impl: plain `Vec<other::Stmt>: m::Tr` E0277s against
//! the bound the caller actually wrote — never an up-front abort, never a `TrRanked` wall.
use decycle::decycle;

pub mod other {
    pub struct Stmt;
}

#[decycle]
mod m {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }

    pub struct Stmt(pub Option<Box<Stmt>>);

    impl Tr for Stmt
    where
        Vec<crate::other::Stmt>: Tr,
    {
        fn f(&self) -> usize {
            1
        }
    }
}

fn main() {}
