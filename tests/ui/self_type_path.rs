//! A self type spelled by a path (`impl Eval for crate::m::Term`) names the item itself, the
//! opt-out, as a path-spelled trait does: the impl takes no part in the cycle and is emitted as
//! written, so the cycle through it overflows as it would without `#[decycle]`.
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Eval {
        fn eval(&self) -> u32;
    }

    pub struct Expr;
    pub struct Term;

    impl Eval for Expr
    where
        Term: Eval,
    {
        fn eval(&self) -> u32 {
            1
        }
    }

    impl Eval for crate::m::Term
    where
        Expr: Eval,
    {
        fn eval(&self) -> u32 {
            2
        }
    }
}

fn main() {}
