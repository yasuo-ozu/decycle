//! A method-level generic whose BOUND mentions `Self` (`F: Fn(&Self)`) cannot be carried across the
//! structural split: the terminator's copy of the method needs `F: Fn(&__ATerm)`, the `__run` decl
//! (implemented for the natural type) needs `F: Fn(&A)`, and one `F` cannot satisfy both. Left
//! alone it produced three `E0277`s whose notes name `__DecycleBody_<nonce>` / `__ATerm_<nonce>`;
//! it is now rejected up-front, on the user's own bound, pointing at the shapes that DO work
//! (`fn(&Self)`, `&dyn Fn(&Self)` — both exercised in `tests/structural_fix_regressions.rs`).
use decycle::decycle;

#[decycle(structural)]
mod m {
    #[decycle]
    pub trait Vis {
        fn apply<F: Fn(&Self) -> i64>(&self, f: F) -> i64;
    }
    pub struct A(pub i64);
    impl Vis for A
    where
        A: Vis,
    {
        fn apply<F: Fn(&Self) -> i64>(&self, f: F) -> i64 {
            f(self)
        }
    }
}

fn main() {}
