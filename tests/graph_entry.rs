//! End-to-end cover for `ranked::process_module_with_graph` (via `tests/graph_bridge`).
//!
//! The bridge derives the graph with `analysis::analyze_module`, feeds it straight back to the
//! graph-taking entry point, and asserts the expansion is byte-identical to the deriving one. This
//! test then exercises the result at runtime, so "identical" also means "works".

#[graph_bridge::decycle_via_graph]
mod ast {
    #[decycle]
    pub trait Tr {
        fn depth(&self) -> usize;
    }

    pub enum Expr {
        Lit,
        Nest(Box<Stmt>),
    }
    pub enum Stmt {
        E(Box<Expr>),
    }

    impl Tr for Expr
    where
        Box<Stmt>: Tr,
    {
        fn depth(&self) -> usize {
            match self {
                Expr::Lit => 0,
                Expr::Nest(s) => s.depth() + 1,
            }
        }
    }

    impl Tr for Stmt
    where
        Box<Expr>: Tr,
    {
        fn depth(&self) -> usize {
            match self {
                Stmt::E(e) => e.depth(),
            }
        }
    }
}

use ast::{Expr, Stmt, Tr};

#[test]
fn the_graph_taking_entry_point_produces_working_code() {
    // ( ( ( Lit ) ) ) — three turns of the Expr <-> Stmt cycle.
    let e = Expr::Nest(Box::new(Stmt::E(Box::new(Expr::Nest(Box::new(Stmt::E(
        Box::new(Expr::Lit),
    )))))));
    assert_eq!(e.depth(), 2);
}

#[test]
fn recursion_runs_past_the_rank_floor() {
    // `recurse_level` is 2 in the bridge; a depth well past it proves the re-entry path is intact.
    let mut e = Expr::Lit;
    for _ in 0..50 {
        e = Expr::Nest(Box::new(Stmt::E(Box::new(e))));
    }
    assert_eq!(e.depth(), 50);
}
