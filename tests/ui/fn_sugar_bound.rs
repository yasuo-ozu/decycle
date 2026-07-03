//! A #[decycle] trait referenced with `Fn(...)`-sugar (`where B: Cb(usize) -> usize`) is not
//! a supported bound form: `TraitReplacer` (the where-clause/body rewriter) carries the
//! original `Parenthesized` arguments onto the ranked-trait replacement path, which has no
//! room left to also insert the Rank argument. This used to silently drop the Rank insertion
//! (producing a malformed `CbRanked<(usize,), Output = usize>` reference that cascaded into
//! confusing, seemingly unrelated errors downstream); it's a clean `abort!` now, matching the
//! identical rejection for this shape written directly on an impl's own trait reference
//! (`PathArgumentsScheme::insert`, unreachable from plain syntax since `impl Trait(..) -> _
//! for X` doesn't parse at all).
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb(usize) -> usize,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

fn main() {}
