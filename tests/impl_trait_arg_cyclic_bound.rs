//! Input-position `impl Trait` in `#[decycle]` trait methods, desugared to method generics on
//! the ranked traits. The headline case here is an `impl Trait` argument whose bound names a
//! *cyclic* (`#[decycle]`) trait defined in the same module — e.g. `fn sink(&self, other: impl
//! Feed, ..)` where `Feed` is decycled. That used to fail (E0276 "impl has stricter requirements
//! than trait" + E0277 `ImplTrait0: Feed` unsatisfied): the desugared method-generic bound was
//! rank-lowered to `FeedRanked<Rank>` in the inductive impl but kept public on the ranked trait
//! definition, and the argument (a value from outside the cycle) can satisfy neither a
//! rank-indexed bound (the same value is threaded through every rank) nor the public trait once
//! the `shadowing_module` dummy shadows the bare name. It now keeps the PUBLIC-trait bound
//! consistently across the ranked trait definition, the inductive impls, the leaf impls, and the
//! re-entry fn.
//!
//! Non-cyclic `impl Trait` bounds (`impl Fn(..)`, HRTB, multiple params, bounded mode) already
//! worked; the guards here keep them working.

#![allow(dead_code)]
use decycle::decycle;

// -------------------------------------------------------------------------------------------
// Headline: `impl CyclicTrait` argument, unbounded mode, driven PAST the floor (exercises the
// leaf + full-height re-entry paths, which carry the desugared method generic too).
// -------------------------------------------------------------------------------------------

#[decycle(recurse_level = 3)]
mod cyclic_bound_m {
    #[decycle]
    pub trait Sink {
        fn sink(&self, other: impl Feed, n: usize) -> usize;
    }
    #[decycle]
    pub trait Feed {
        fn feed(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Sink for A
    where
        B: Sink,
        B: Feed,
    {
        fn sink(&self, other: impl Feed, n: usize) -> usize {
            if n == 0 {
                other.feed(1)
            } else {
                B.sink(other, n - 1) + 1
            }
        }
    }
    impl Feed for B
    where
        A: Feed,
    {
        fn feed(&self, n: usize) -> usize {
            n + 1
        }
    }
    impl Feed for A
    where
        B: Feed,
    {
        fn feed(&self, n: usize) -> usize {
            n + 2
        }
    }
    impl Sink for B
    where
        A: Sink,
        A: Feed,
    {
        fn sink(&self, other: impl Feed, n: usize) -> usize {
            if n == 0 {
                other.feed(1)
            } else {
                A.sink(other, n - 1) + 1
            }
        }
    }
}

#[test]
fn impl_cyclic_trait_arg_within_level() {
    use cyclic_bound_m::Sink as _;
    // n = 2 < recurse_level: pure inductive path. Unwinds two "+1", then B.feed(1) = 2.
    assert_eq!(cyclic_bound_m::A.sink(cyclic_bound_m::B, 2), 4);
}

#[test]
fn impl_cyclic_trait_arg_past_floor() {
    use cyclic_bound_m::Sink as _;
    // n = 20 > recurse_level: crosses the floor and re-enters at full height, threading the
    // `impl Feed` value through every re-entry. Unwinds twenty "+1", then B.feed(1) = 2.
    assert_eq!(cyclic_bound_m::A.sink(cyclic_bound_m::B, 20), 22);
}

// -------------------------------------------------------------------------------------------
// `impl CyclicTrait` where the cyclic trait is GENERIC (`impl Feed<u8>`): the bound's own
// type argument must survive qualification untouched.
// -------------------------------------------------------------------------------------------

#[decycle(recurse_level = 2)]
mod generic_cyclic_bound_m {
    #[decycle]
    pub trait Sink {
        fn sink(&self, other: impl Feed<u8>, n: usize) -> usize;
    }
    #[decycle]
    pub trait Feed<T> {
        fn feed(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Sink for A
    where
        B: Sink,
        B: Feed<u8>,
    {
        fn sink(&self, other: impl Feed<u8>, n: usize) -> usize {
            if n == 0 {
                other.feed(1)
            } else {
                B.sink(other, n - 1) + 1
            }
        }
    }
    impl Feed<u8> for B
    where
        A: Feed<u8>,
    {
        fn feed(&self, n: usize) -> usize {
            n + 1
        }
    }
    impl Feed<u8> for A
    where
        B: Feed<u8>,
    {
        fn feed(&self, n: usize) -> usize {
            n + 5
        }
    }
    impl Sink for B
    where
        A: Sink,
        A: Feed<u8>,
    {
        fn sink(&self, other: impl Feed<u8>, n: usize) -> usize {
            if n == 0 {
                other.feed(1)
            } else {
                A.sink(other, n - 1) + 1
            }
        }
    }
}

#[test]
fn impl_generic_cyclic_trait_arg_past_floor() {
    use generic_cyclic_bound_m::Sink as _;
    // 15 "+1" on unwind, then other (B).feed(1) = 1 + 1 = 2.
    assert_eq!(generic_cyclic_bound_m::A.sink(generic_cyclic_bound_m::B, 15), 17);
}

// -------------------------------------------------------------------------------------------
// Regression guards: non-cyclic `impl Trait` bounds keep working.
// -------------------------------------------------------------------------------------------

// Bounded mode (`support_infinite_cycle = false`): no runtime machinery, `impl Fn` still
// desugars to a method generic and delegates within the limit.
#[decycle(recurse_level = 8, support_infinite_cycle = false)]
mod bounded_m {
    #[decycle]
    pub trait Fold {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Fold for A
    where
        B: Fold,
    {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize {
            if n == 0 {
                f(0)
            } else {
                B.fold(f, n - 1) + 1
            }
        }
    }
    impl Fold for B
    where
        A: Fold,
    {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize {
            if n == 0 {
                f(0)
            } else {
                A.fold(f, n - 1) + 1
            }
        }
    }
}

#[test]
fn bounded_mode_impl_fn_arg() {
    use bounded_m::Fold as _;
    assert_eq!(bounded_m::A.fold(|v| v + 7, 3), 3 + 7);
}

// Multiple `impl Trait` params, one carrying an HRTB.
#[decycle(recurse_level = 2)]
mod multi_m {
    #[decycle]
    pub trait Comb {
        fn comb(
            &self,
            f: impl Fn(usize) -> usize,
            g: impl for<'a> Fn(&'a usize) -> usize,
            n: usize,
        ) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Comb for A
    where
        B: Comb,
    {
        fn comb(
            &self,
            f: impl Fn(usize) -> usize,
            g: impl for<'a> Fn(&'a usize) -> usize,
            n: usize,
        ) -> usize {
            if n == 0 {
                f(0) + g(&0)
            } else {
                B.comb(f, g, n - 1) + 1
            }
        }
    }
    impl Comb for B
    where
        A: Comb,
    {
        fn comb(
            &self,
            f: impl Fn(usize) -> usize,
            g: impl for<'a> Fn(&'a usize) -> usize,
            n: usize,
        ) -> usize {
            if n == 0 {
                f(0) + g(&0)
            } else {
                A.comb(f, g, n - 1) + 1
            }
        }
    }
}

#[test]
fn multi_impl_trait_args_with_hrtb_past_floor() {
    use multi_m::Comb as _;
    // Closures cannot be encoded as a re-entry key (see `__reentry::assert_key_encodable`);
    // coerce to function pointers, which are uniquely named.
    assert_eq!(
        multi_m::A.comb(
            (|v| v + 1) as fn(usize) -> usize,
            (|v: &usize| *v + 1) as fn(&usize) -> usize,
            5
        ),
        5 + 2
    );
}
