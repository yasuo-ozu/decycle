//! A WRAPPED cyclic bound (`Box<Stmt>: Tr`) can't be rank-lowered — `Box<Stmt>: TrRanked<Rank>` has
//! no floor impl for the container, so rustc would otherwise emit a raft of raw `TrRanked<…>` overflow
//! errors at the useless module span. Ranked now rejects it up-front with a legible message on the
//! user's own `where`-bound, pointing at `#[decycle(structural)]` (which forwards it via a blanket
//! `impl<T: Tr> Tr for Box<T>`). Mirrors the structural `wrapped_bound_not_forwarded.rs` diagnostic.
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
