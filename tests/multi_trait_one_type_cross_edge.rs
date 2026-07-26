//! MULTIPLE participating traits over the same SCC, with a **cross-TRAIT** cyclic edge: `Expr`'s `Eval`
//! body reaches `Expr`'s `Size` and vice-versa (`Expr: Eval → Expr: Size → Expr: Eval`). Run under BOTH
//! algorithms — the structural side exercises the `(type, trait)`-pair obligation graph.
mod common;

dual_mod! {
    multi_trait {
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
                if n == 0 { 0 } else { self.size(n - 1) + 1 } // cross-trait hop Eval -> Size
            }
        }

        impl Size for Expr
        where
            Expr: Eval,
        {
            fn size(&self, n: usize) -> usize {
                if n == 0 { 0 } else { self.eval(n - 1) + 1 } // cross-trait hop back Size -> Eval
            }
        }
    }
}

#[test]
fn cross_trait_cycle_on_one_type_is_unbounded() {
    on_both!(multi_trait, {
        // Alternates Eval/Size on every hop, well past any fixed depth.
        assert_eq!(Expr.eval(2000), 2000);
        assert_eq!(Expr.size(2001), 2001);
    });
}
