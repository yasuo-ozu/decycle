//! A NON-REGULAR ("growing") cyclic where-bound — the bound's type argument grows one
//! wrapper per cycle edge (`A<Vec<X>>: Tr<Vec<X>>` on `impl<X> Tr<X> for A<X>`), so the
//! reachability walk's obligations never close. Before the cap this diverged inside the
//! macro until rustc crashed with SIGSEGV. Must be a clean, actionable abort instead.
//! (The body deliberately never makes the growing call: the walk keys off the BOUND alone,
//! and a recursing body would add rustc's own separate E0275 on the fallback emission.)
use decycle::decycle;

#[decycle]
mod m {
    use std::marker::PhantomData;

    #[decycle]
    pub trait Tr<X> {
        fn go(&self, n: usize) -> usize;
    }

    pub struct A<X>(pub PhantomData<X>);

    impl<X> Tr<X> for A<X>
    where
        A<Vec<X>>: Tr<Vec<X>>,
    {
        fn go(&self, n: usize) -> usize {
            n
        }
    }
}

fn main() {}
