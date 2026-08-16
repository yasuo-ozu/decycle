//! A cycle that travels through a container which IMPLEMENTS the cyclic trait itself.
//!
//! `Expr: Tr` is stated as `Wrap<Stmt>: Tr`, and `Wrap` carries a blanket `impl<T: Tr> Tr for
//! Wrap<T>`, so the obligation walks `Expr -> Wrap<Stmt> -> Stmt -> Wrap<Expr> -> Expr` and
//! overflows (E0275: "required for `Wrap<Stmt>` to implement `Tr`") without decycle. The structural
//! engine breaks it, and this file pins that it does — it compiles and it runs.
//!
//! It is the working program behind `decycle-impl/tests/analysis_regressions.rs`, whose obligation
//! graph used to come back ACYCLIC for exactly this module: `Wrap` is a participant in its own
//! right, and a bound whose head was a participant swallowed the nested `Stmt`/`Expr` endpoints —
//! so `cyclic_subgraph` had nothing to hand an engine and the graph-fed run emitted no terminators.

#[decycle::decycle(structural)]
mod structural {
    #[decycle]
    pub trait Tr {
        fn depth(&self) -> u32;
    }

    pub struct Wrap<T>(pub T);

    pub enum Expr {
        Lit,
        Nest(Box<Stmt>),
    }
    pub enum Stmt {
        E(Box<Expr>),
    }

    impl<T: Tr> Tr for Wrap<T> {
        fn depth(&self) -> u32 {
            self.0.depth()
        }
    }

    impl Tr for Expr
    where
        Wrap<Stmt>: Tr,
    {
        fn depth(&self) -> u32 {
            match self {
                Expr::Lit => 0,
                Expr::Nest(s) => s.depth() + 1,
            }
        }
    }

    impl Tr for Stmt
    where
        Wrap<Expr>: Tr,
    {
        fn depth(&self) -> u32 {
            match self {
                Stmt::E(e) => e.depth(),
            }
        }
    }
}

// NOTE: only the STRUCTURAL engine is exercised here. The same module under the default ranked
// engine still overflows (`Wrap<Expr>: TrRanked<()>`): the blanket `impl<T: Tr> Tr for Wrap<T>` is
// not itself rank-lowered, so the rank chain runs through an un-lowered link. That is a pre-existing
// ranked-engine limitation, unrelated to the obligation graph this file backs, and left as is.

macro_rules! check {
    ($m:ident) => {{
        use $m::{Expr, Stmt, Tr, Wrap};
        // ( ( ( Lit ) ) ) — three turns of the Expr <-> Stmt cycle.
        let e = Expr::Nest(Box::new(Stmt::E(Box::new(Expr::Nest(Box::new(Stmt::E(
            Box::new(Expr::Lit),
        )))))));
        assert_eq!(e.depth(), 2);
        // The container's own impl is still usable, and still forwards.
        assert_eq!(Wrap(Expr::Lit).depth(), 0);
        assert_eq!(Wrap(Stmt::E(Box::new(Expr::Lit))).depth(), 0);
    }};
}

#[test]
fn the_structural_engine_breaks_a_cycle_through_a_participating_container() {
    check!(structural);
}
