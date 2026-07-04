//! D4 (scoping check): the SAME return-`impl Trait` shape that aborts under (default-on)
//! `support_infinite_cycle` still compiles fine when it's turned off — `emit_reentry_items` (and
//! its D4 `abort!`) only runs under `support_infinite_cycle`, so a bounded cycle just delegates
//! to the real method (and, at the rank floor, `unimplemented!()`s) verbatim.
//!
//! The return bound is deliberately `impl std::fmt::Debug` rather than `impl Iterator<…>`: the
//! bounded-mode rank floor's body is *always* `unimplemented!()` (never returns), and
//! return-position `impl Trait` in a trait method (RPITIT) infers each impl's own hidden type
//! independently — a diverging-only body's hidden type infers as `()`, so the bound must be one
//! `()` satisfies. (This is a plain, decycle-independent property of RPITIT + a
//! never-returning body, reproducible with no macros at all; it is not a D4 concern.)
use decycle::decycle;

#[decycle(support_infinite_cycle = false)]
mod m {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> impl std::fmt::Debug;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> impl std::fmt::Debug {
            B.cb(n)
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            n
        }
    }
}

fn main() {
    use m::Ca;
    let _ = m::A.ca(3);
}
