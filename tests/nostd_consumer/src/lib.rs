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
//! It also announces a trait (`#[announce]` with `decycle::check`, through the `announce` feature)
//! and brings it into a module of each kind with `#[decycle] use`: the announcing side builds for a
//! core-only target too.
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

/// A trait announced with `decycle::check`, in a `no_std` crate. `announce` is built here without its
/// `proc-macro-api` feature, so this exercises `#[announce]` and the check on a core-only target.
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
pub trait Announced {
    fn depth(&self) -> u32;
}

/// The same trait, announced for the structural engine (`decycle::check(structural)`).
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check(structural))]
pub trait AnnouncedStructural<T = u32> {
    fn depth(&self, t: T) -> u32;
}

/// `#[decycle] use` of announced traits, in both engines.
#[decycle(support_infinite_cycle = false)]
pub mod ranked_announced {
    #[decycle]
    use super::Announced;

    pub struct A(pub u32);
    pub struct B(pub u32);

    impl Announced for A
    where
        B: Announced,
    {
        fn depth(&self) -> u32 {
            self.0
        }
    }

    impl Announced for B
    where
        A: Announced,
    {
        fn depth(&self) -> u32 {
            self.0
        }
    }
}

#[decycle(structural)]
pub mod structural_announced {
    #[decycle]
    use super::AnnouncedStructural;

    pub struct A(pub u32);
    pub struct B(pub u32);

    impl AnnouncedStructural for A
    where
        B: AnnouncedStructural,
    {
        fn depth(&self, t: u32) -> u32 {
            self.0 + t
        }
    }

    impl AnnouncedStructural for B
    where
        A: AnnouncedStructural,
    {
        fn depth(&self, t: u32) -> u32 {
            self.0 + t
        }
    }
}
