//! Ported from cycling:main tests/p4_multi_trait.rs — MULTIPLE participating
//! traits over the same SCC, with a cross-TRAIT cyclic edge.
//!
//! Existing decycle tests exercise cross-trait cycles between DIFFERENT self
//! types (the README's `Left: A` / `Right: B` shape, `advanced_cycles.rs`'s
//! `CycleA`/`CycleB`). This test covers the shape p4_multi_trait is really about:
//! ONE self type implementing TWO `#[decycle]` traits, whose cyclic obligation
//! crosses from one trait to the OTHER on the same type (`Expr`'s `Eval` body
//! reaches for `Expr`'s `Size`, and vice-versa) — the two ranked engines must
//! share the module and drive each other past the floor.

use decycle::decycle;

#[decycle]
mod multi_trait {
    #[decycle]
    pub trait Eval {
        fn eval(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Size {
        fn size(&self, n: usize) -> usize;
    }

    pub struct Expr;

    impl Eval for Expr
    where
        Expr: Size,
    {
        fn eval(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                // Cross-trait hop: Eval -> Size on the same type.
                self.size(n - 1) + 1
            }
        }
    }

    impl Size for Expr
    where
        Expr: Eval,
    {
        fn size(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                // Cross-trait hop back: Size -> Eval on the same type.
                self.eval(n - 1) + 1
            }
        }
    }
}

#[test]
fn cross_trait_cycle_on_one_type_is_unbounded() {
    use multi_trait::{Eval, Size};
    // Alternates Eval/Size on every hop, well past the default recurse_level (10).
    assert_eq!(multi_trait::Expr.eval(2000), 2000);
    assert_eq!(multi_trait::Expr.size(2001), 2001);
}
