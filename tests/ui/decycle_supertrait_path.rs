//! A `#[decycle]` supertrait named by a path (`crate::m::Base`) is the opt-out: the ranked twin
//! keeps the real trait as written, rather than the supertrait's ranked twin. The impls still prove
//! it, but a subtrait impl's body re-emitted by the engine can reach `Base`'s methods only through
//! that path: by method syntax the call has no rank to resolve at (E0283; E0275 on newer rustc).
//! (`Base` or `self::Base` ranks the supertrait with the trait: `tests/decycle_supertraits.rs`.)
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Base {
        fn base(&self) -> u32;
    }

    #[decycle]
    pub trait Eval: crate::m::Base {
        fn eval(&self) -> u32;
    }

    pub struct Expr(pub Option<Box<Term>>);
    pub struct Term(pub Option<Box<Expr>>);

    impl Base for Expr
    where
        Term: Base,
    {
        fn base(&self) -> u32 {
            self.0.as_ref().map_or(1, |t| t.base())
        }
    }

    impl Base for Term
    where
        Expr: Base,
    {
        fn base(&self) -> u32 {
            self.0.as_ref().map_or(1, |e| e.base())
        }
    }

    impl Eval for Expr
    where
        Term: Eval,
    {
        fn eval(&self) -> u32 {
            self.base() + self.0.as_ref().map_or(0, |t| t.eval())
        }
    }

    impl Eval for Term
    where
        Expr: Eval,
    {
        fn eval(&self) -> u32 {
            self.0.as_ref().map_or(0, |e| e.eval())
        }
    }
}

fn main() {}
