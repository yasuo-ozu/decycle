//! `no_std` fixture: a consumer that actually **expands `#[decycle]`** with default features off.
//!
//! The `no_std` CI job used to build only the facade crate, which contains no expansions at all —
//! so it proved that `decycle` itself compiles for a core-only target, and nothing whatsoever about
//! the code the macro emits. Every generated-code path was therefore verified on `std` only.
//!
//! This crate covers the two combinations documented as supported without `std`:
//!
//!   * the structural engine, which emits no runtime machinery at all; and
//!   * the ranked engine with `support_infinite_cycle = false`, which needs no re-entry registry
//!     (the registry is a `thread_local!`, hence `std`-only).
//!
//! Unbounded ranked without `std` is *expected* to fail, with a diagnostic naming
//! `UnboundedReentryRequiresTheDecycleStdFeature`. That case is pinned by a trybuild snapshot
//! rather than here, since this crate has to build.

#![no_std]

use decycle::decycle;

/// Structural engine: a classic `Expr` ↔ `Stmt` AST cycle.
#[decycle(structural)]
pub mod structural_cycle {
    #[decycle]
    pub trait Eval {
        fn eval(&self) -> u32;
    }

    pub struct Expr(pub u32, pub Option<&'static Stmt>);
    pub struct Stmt(pub u32, pub Option<&'static Expr>);

    impl Eval for Expr
    where
        Stmt: Eval,
    {
        fn eval(&self) -> u32 {
            self.0 + self.1.map(|s| s.eval()).unwrap_or(0)
        }
    }

    impl Eval for Stmt
    where
        Expr: Eval,
    {
        fn eval(&self) -> u32 {
            self.0 + self.1.map(|e| e.eval()).unwrap_or(0)
        }
    }
}

/// Ranked engine, bounded — the spelling that does not need the registry.
#[decycle(support_infinite_cycle = false)]
pub mod ranked_bounded {
    #[decycle]
    pub trait Walk {
        fn walk(&self) -> u32;
    }

    pub struct A(pub u32);
    pub struct B(pub u32);

    impl Walk for A
    where
        B: Walk,
    {
        fn walk(&self) -> u32 {
            self.0
        }
    }

    impl Walk for B
    where
        A: Walk,
    {
        fn walk(&self) -> u32 {
            self.0
        }
    }
}
