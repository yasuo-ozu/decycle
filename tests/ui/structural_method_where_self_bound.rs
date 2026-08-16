//! The `where`-clause spelling of `tests/ui/structural_method_generic_self_bound.rs`, plus the case
//! where only the trait's DECLARATION mentions `Self`: the impl writes the bound concretely
//! (`F: Fn(&A)`), which cannot satisfy the terminator's copy either. Both are rejected by reading
//! the declaration alongside the impl.
//!
//! (`where Self: Sized` — a method-level predicate that *bounds* `Self` rather than mentioning it in
//! a bound — is deliberately still allowed; `keep` below is here so the rejection has to walk past
//! it, and `structural_fix_regressions.rs` pins that such a method still compiles and runs.)
use decycle::decycle;

#[decycle(structural)]
mod m {
    #[decycle]
    pub trait Vis {
        fn apply<F>(&self, f: F) -> i64
        where
            F: Fn(&Self) -> i64;
        fn keep(&self) -> i64
        where
            Self: Sized;
    }
    pub struct A(pub i64);
    impl Vis for A
    where
        A: Vis,
    {
        // Spelled concretely: legal Rust here, unprovable on the terminator.
        fn apply<F>(&self, f: F) -> i64
        where
            F: Fn(&A) -> i64,
        {
            f(self)
        }
        fn keep(&self) -> i64
        where
            Self: Sized,
        {
            self.0
        }
    }
}

fn main() {}
