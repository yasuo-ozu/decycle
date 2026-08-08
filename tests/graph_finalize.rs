//! `finalize_with_graph` — the same graph + `contract` wiring at the lower-level entry point, for a
//! wrapper macro that holds `FinalizeArgs` rather than an `ItemMod`.

#[graph_bridge::finalize_via_graph_contract]
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
fn finalize_with_graph_respells_and_ranks() {
    let mut e = Expr::Lit;
    for _ in 0..25 {
        e = Expr::Nest(Box::new(Stmt::E(Box::new(e))));
    }
    assert_eq!(e.depth(), 25);
}
