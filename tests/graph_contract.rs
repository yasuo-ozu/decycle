//! `GraphOptions { contract: true }`: the graph is the only statement of the cycle.
//!
//! Every trait path in the module below is **fully qualified** — header and bound alike — which is how
//! a generating caller naturally emits them, and which the engine otherwise reads as "ordinary
//! premise, leave alone". Nothing here says `Expr` and `Stmt` recurse except the participant set the
//! bridge hands over, so this compiles only if `contract` re-spells the impls from that set.

#[graph_bridge::decycle_via_graph_contract]
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

    impl crate::ast::Tr for Expr
    where
        Box<Stmt>: crate::ast::Tr,
    {
        fn depth(&self) -> usize {
            match self {
                Expr::Lit => 0,
                Expr::Nest(s) => s.depth() + 1,
            }
        }
    }

    impl crate::ast::Tr for Stmt
    where
        Box<Expr>: crate::ast::Tr,
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
fn contract_respells_a_fully_qualified_cycle() {
    let mut e = Expr::Lit;
    for _ in 0..30 {
        e = Expr::Nest(Box::new(Stmt::E(Box::new(e))));
    }
    assert_eq!(e.depth(), 30);
}
