//! Regression tests for `decycle-impl/src/ranked/finalize.rs`.
//!
//! Every test here pins a defect that was reproduced before the fix landed: each cycle below
//! panicked (`re-entry fn not registered`) or failed to compile on the parent commit, and the
//! assertions check the actual computed value, not merely that expansion succeeded.

// The non-entry traits of each cycle are only ever reached through their ranked variants.
#![allow(dead_code)]

use decycle::decycle;

// ---------------------------------------------------------------------------------------------
// (2) A cross-edge (rule 2) to a GENERIC cyclic method used to be skipped outright, so the
// callee's floor found no registration and the fail-closed lookup panicked. The two cycles below
// are byte-identical apart from the method's `<T: Copy>`; both must return the same value.
// `recurse_level = 1` is what makes this observable: at level >= 2 the callee's own inductive
// frame runs (and rule 1 registers it) before any floor is crossed.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 1)]
mod plain_l1 {
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
        B: Cb,
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

#[decycle(recurse_level = 1)]
mod generic_l1 {
    #[decycle]
    pub trait Ca {
        fn ca<T: Copy>(&self, t: T, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb<T: Copy>(&self, t: T, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca<T: Copy>(&self, t: T, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(t, n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb<T: Copy>(&self, t: T, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(t, n - 1) + 1
            }
        }
    }
}

#[test]
fn generic_cross_edge_matches_its_non_generic_twin() {
    use generic_l1::Ca as _;
    use plain_l1::Ca as _;
    assert_eq!(plain_l1::A.ca(20), 20);
    assert_eq!(generic_l1::A.ca(0u8, 20), 20);
    // Distinct instantiations stay on distinct keys past the floor.
    assert_eq!(generic_l1::A.ca(0u64, 21), 21);
}

// ---------------------------------------------------------------------------------------------
// (3) A cyclic bound spelled `where Self: OtherTrait` used to lose its cross-edge registration:
// `cyclic_where_bounds` kept the literal `Self` as the obligation target, which no candidate
// impl's self type ever unified with, so the reachability check rejected the bound and the
// registration was dropped. The concrete spelling of the same cycle always worked.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 1)]
mod named_self_bound {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    impl Ca for A
    where
        A: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.cb(n - 1) + 1
            }
        }
    }
    impl Cb for A
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

#[decycle(recurse_level = 1)]
mod keyword_self_bound {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    impl Ca for A
    where
        Self: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.cb(n - 1) + 1
            }
        }
    }
    impl Cb for A
    where
        Self: Ca,
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

#[test]
fn self_spelled_cyclic_bound_matches_its_named_twin() {
    use keyword_self_bound::Ca as _;
    use named_self_bound::Ca as _;
    assert_eq!(named_self_bound::A.ca(20), 20);
    assert_eq!(keyword_self_bound::A.ca(20), 20);
}

// ---------------------------------------------------------------------------------------------
// (4) `fingerprint_expr` folded `size_of::<Self>()` unconditionally, so an impl on a
// conditionally-sized self type (`impl<T: ?Sized> Ca for Wrap<T>` — valid Rust, and fine without
// the macro) failed with E0277 at the attribute. The layout fold must be skipped whenever the
// target may be unsized.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 1)]
mod maybe_sized_self {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct Wrap<T: ?Sized> {
        pub tail: T,
    }
    pub struct B;
    impl<T: ?Sized> Ca for Wrap<T>
    where
        B: Cb,
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
        Wrap<u32>: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                Wrap { tail: 0u32 }.ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn maybe_sized_self_type_expands_and_runs() {
    use maybe_sized_self::Ca as _;
    assert_eq!(maybe_sized_self::Wrap { tail: 0u32 }.ca(20), 20);
    // The unsized instantiation the `?Sized` bound exists for.
    let unsized_wrap: &maybe_sized_self::Wrap<[u8]> = &maybe_sized_self::Wrap { tail: [0u8; 4] };
    assert_eq!(unsized_wrap.ca(7), 7);
}
