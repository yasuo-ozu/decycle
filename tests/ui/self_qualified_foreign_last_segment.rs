//! The `self::`-qualified twin of `peel_foreign_last_segment.rs`, and the second half of the
//! same last-segment confusion.
//!
//! `crate::other::Stmt` is a DIFFERENT item from the module's own `Stmt`, so a cyclic bound
//! targeting it has no ranked impl to descend through. `validate_impl_where_bounds` decided
//! otherwise — its `head_ok` compared the target's LAST path segment against the cycle heads —
//! so the bound passed the up-front check and `finalize` went on to rank-lower a foreign type,
//! producing a per-rank wall of unprovable `TrRanked<((((..>` errors at the attribute span.
//!
//! The `self::` spelling is the one that must abort: `remove_cyclic_bounds` classifies it as
//! cyclic regardless of its target (it is depth-fragile), so the rewrite rank-lowers it no matter
//! what the head is. This snapshot pins the single legible rejection that replaces the wall.
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
        crate::other::Stmt: self::Tr,
    {
        fn f(&self) -> usize {
            1
        }
    }
}

fn main() {}
