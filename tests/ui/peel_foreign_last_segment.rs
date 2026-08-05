//! A cyclic bound whose target is a FOREIGN type sharing its last path segment with a local cycle
//! head. `crate::other::Stmt` is a different item from the module's own `Stmt`, so it must not be
//! peeled as a cycle member.
//!
//! Peeling it (matching on the last path segment) produced rank-lowering obligations against the
//! foreign type — 14 errors, a per-rank wall of `other::Stmt: TrRanked<((((..` at the attribute
//! span. Now the target is treated as an outer type in its entirety, nothing is peeled, and the
//! bound is reported once against what the caller actually wrote.
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
