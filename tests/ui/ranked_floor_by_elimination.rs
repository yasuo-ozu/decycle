//! A call to a cycle trait's method whose rank nothing pins must not compile. `self.base()` in an
//! impl of ANOTHER trait, with no `Self: Base` bound, leaves the rank of `Expr: BaseRanked<_>` to
//! inference, and the only impls left for it are the floors. The next-generation trait solver
//! (nightly's default) rules the step impl `BaseRanked<(R,)>` out, through the `Term: Base` bound,
//! and used to select the floor `BaseRanked<()>` by elimination: this program compiled and panicked
//! at run time ("re-entry fn not registered", or "cycle limit reached" with
//! `support_infinite_cycle = false`). The floor's decoy twin keeps a second impl in play, so every
//! solver reports the ambiguity, `E0283`, at the call. (The old solver, on which this snapshot is
//! blessed, reported it already.) The fixes: a `Self: Base` bound, `trait Eval: Base`, or the call
//! by path `crate::m::Base::base(self)` (`tests/floor_by_elimination.rs`).
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Base {
        fn base(&self) -> u32;
    }
    #[decycle]
    pub trait Eval {
        fn eval(&self) -> u32;
    }
    pub struct Expr(pub Option<Box<Term>>);
    pub struct Term(pub Option<Box<Expr>>);
    impl Base for Expr
    where
        Term: Base,
    {
        fn base(&self) -> u32 {
            self.0.as_ref().map_or(1, |t| t.base() + 1)
        }
    }
    impl Base for Term
    where
        Expr: Base,
    {
        fn base(&self) -> u32 {
            self.0.as_ref().map_or(1, |e| e.base() + 1)
        }
    }
    impl Eval for Expr
    where
        Term: Base,
    {
        fn eval(&self) -> u32 {
            self.base()
        }
    }
}

fn main() {
    use m::Eval;
    m::Expr(None).eval();
}
