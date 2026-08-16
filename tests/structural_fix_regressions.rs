//! Structural-engine defect fixes that are *behavioral* — each one used to fail to compile, or to
//! run the wrong number of times. (The rejections that go with them live in `tests/ui/`.)
//!
//!  * an impl that spells the concrete self type where the trait declares `Self`,
//!  * `Self::CONST` inside a cyclic body,
//!  * a method attribute macro expanded once, not once per generated copy,
//!  * the callback shapes the engine's own diagnostics recommend (`fn(&Self)`, `&dyn Fn(&Self)`).
//!
//! Everything that is not specific to one engine runs under BOTH (`dual_mod!`/`on_both!`), because
//! the fixes must not make the two disagree.
#![allow(dead_code)]

mod common;

// ===== 1. An impl may spell the concrete self type where the trait declares `Self` =====
//
// `fn merge(&self, other: &A)` implementing `fn merge(&self, other: &Self)` is legal Rust — inside
// `impl Vis for A` the two spellings are the same type. But the structural engine re-emits that
// signature on the terminator, where `Self` is `__ATerm`, so the copied `&A` mismatched the trait:
// `E0053` + `E0308`, with rustc "helpfully" suggesting the user write `&__ATerm_<nonce>` themselves.
// The impl signature is now canonicalized against the trait's declaration first.
dual_mod! {
    concrete_self_param {
        #[decycle]
        pub trait Vis {
            fn merge(&self, other: &Self) -> i64;
            /// Nested inside a container, so the rewrite has to descend generic arguments.
            fn pick(&self, other: Option<&Self>) -> i64;
            /// And through the return type.
            fn dup(&self) -> Box<Self>;
        }
        pub struct A(pub i64, pub Box<B>);
        pub struct B(pub i64);

        impl Vis for A
        where
            B: Vis,
        {
            // Spelled concretely — this is the shape that used to fail.
            fn merge(&self, other: &A) -> i64 {
                self.0 + other.0
            }
            fn pick(&self, other: Option<&A>) -> i64 {
                other.map_or(0, |o| o.0)
            }
            fn dup(&self) -> Box<A> {
                Box::new(A(self.0, Box::new(B(self.1 .0))))
            }
        }
        impl Vis for B
        where
            A: Vis,
        {
            // The `Self` spelling of the very same trait must keep working next to it.
            fn merge(&self, other: &Self) -> i64 {
                self.0 + other.0
            }
            fn pick(&self, other: Option<&Self>) -> i64 {
                other.map_or(0, |o| o.0)
            }
            fn dup(&self) -> Box<Self> {
                Box::new(B(self.0))
            }
        }
    }
}

#[test]
fn impl_may_spell_the_concrete_self_type_where_the_trait_says_self() {
    on_both!(concrete_self_param, {
        let a = A(1, Box::new(B(2)));
        assert_eq!(a.merge(&A(10, Box::new(B(0)))), 11);
        assert_eq!(a.pick(Some(&A(5, Box::new(B(0))))), 5);
        assert_eq!(a.pick(None), 0);
        assert_eq!(a.dup().0, 1);
        assert_eq!(B(3).merge(&B(4)), 7);
        assert_eq!(B(3).dup().0, 3);
    });
}

// The same, on a GENERIC cycle member (`&G<T>` for a declared `&Self`) and next to an associated
// const, so the rewrite is pinned where the self type carries arguments and where the body impl has
// to prove the real trait's `impl` for itself.
dual_mod! {
    concrete_self_param_generic {
        #[decycle]
        pub trait Vis {
            const K: i64;
            fn merge(&self, other: &Self) -> i64;
        }
        pub struct G<T>(pub T, pub Box<H<T>>);
        pub struct H<T>(pub T);

        impl<T: Copy + Into<i64>> Vis for G<T>
        where
            H<T>: Vis,
        {
            const K: i64 = 100;
            fn merge(&self, other: &G<T>) -> i64 {
                Self::K + self.0.into() + other.0.into()
            }
        }
        impl<T: Copy + Into<i64>> Vis for H<T>
        where
            G<T>: Vis,
        {
            const K: i64 = 200;
            fn merge(&self, other: &Self) -> i64 {
                Self::K + self.0.into() + other.0.into()
            }
        }
    }
}

#[test]
fn concrete_self_spelling_on_a_generic_member() {
    on_both!(concrete_self_param_generic, {
        let g = G(1i32, Box::new(H(2i32)));
        assert_eq!(g.merge(&G(10i32, Box::new(H(0i32)))), 111);
        assert_eq!(H(3i32).merge(&H(4i32)), 207);
    });
}

// ===== 2. …but a concrete type the TRAIT declares is NOT a `Self` position =====
//
// The rewrite is driven by the declaration precisely so this case survives: `feed` genuinely takes
// an `&A`, whatever the impl's own self type is, and must be forwarded untouched (no terminator
// cast, no re-spelling).
dual_mod! {
    concrete_decl_param {
        #[decycle]
        pub trait Feed {
            fn feed(&self, a: &A) -> i64;
        }
        pub struct A(pub i64, pub Box<B>);
        pub struct B(pub i64);

        impl Feed for A
        where
            B: Feed,
        {
            fn feed(&self, a: &A) -> i64 {
                self.0 + a.0
            }
        }
        impl Feed for B
        where
            A: Feed,
        {
            fn feed(&self, a: &A) -> i64 {
                self.0 + a.0
            }
        }
    }
}

#[test]
fn a_concrete_parameter_the_trait_declares_is_left_alone() {
    on_both!(concrete_decl_param, {
        let a = A(1, Box::new(B(2)));
        assert_eq!(a.feed(&A(10, Box::new(B(0)))), 11);
        assert_eq!(B(5).feed(&a), 6);
    });
}

// ===== 3. `Self::CONST` inside a cyclic method body =====
//
// The body runs inside `impl __DecycleBody for A`, and `A` implements the real trait in that same
// scope. Declaring the associated CONST on `__DecycleBody` too made every `Self::K` ambiguous
// (`E0034: multiple applicable items in scope`, one candidate named `__DecycleBody_<nonce>`), even
// though the README lists associated consts as supported. Associated TYPES are still declared there
// (they are unambiguous, and the projection resolver reads them) — `out`/`Self::Out` pins that.
dual_mod! {
    assoc_const_body {
        #[decycle]
        pub trait Vis {
            const K: i64;
            type Out;
            fn v(&self) -> i64;
            fn out(&self) -> Self::Out;
        }
        pub struct A(pub i64, pub Box<B>);
        pub struct B(pub i64);

        impl Vis for A
        where
            B: Vis,
        {
            const K: i64 = 3;
            type Out = i64;
            fn v(&self) -> i64 {
                let x: Self::Out = Self::K;
                x + self.0 + self.1.v()
            }
            fn out(&self) -> Self::Out {
                Self::K
            }
        }
        impl Vis for B
        where
            A: Vis,
        {
            const K: i64 = 4;
            type Out = i64;
            fn v(&self) -> i64 {
                Self::K + self.0
            }
            fn out(&self) -> Self::Out {
                Self::K
            }
        }
    }
}

#[test]
fn associated_const_resolves_inside_a_cyclic_body() {
    on_both!(assoc_const_body, {
        // A::v = K(3) + 1 + B::v, B::v = K(4) + 2
        assert_eq!(A(1, Box::new(B(2))).v(), 10);
        assert_eq!(B(2).v(), 6);
        assert_eq!(A(1, Box::new(B(2))).out(), 3);
        assert_eq!(B(2).out(), 4);
        // and from outside the cycle, through the real trait
        assert_eq!(<A as Vis>::K, 3);
        assert_eq!(<B as Vis>::K, 4);
    });
}

// ===== 4. The callback shapes the diagnostics recommend actually work =====
//
// A method generic whose bound mentions `Self` (`F: Fn(&Self)`) is rejected — see
// `tests/ui/structural_method_generic_self_bound.rs` — and two of the engine's own error texts used
// to recommend exactly that. They now point at `fn(&Self)` and `&dyn Fn(&Self)`; this pins that
// both of those really do work, so the advice stays true.
//
// Structural-only, deliberately: the same module under `#[decycle]` does not compile today (the
// ranked engine's `Self`-substituted trait declaration loses the elided higher-ranked lifetime —
// "expected `for<'a> fn(&'a _)`, found `fn(&'__dcl1 _)`"). That is a pre-existing ranked-engine
// limitation, untouched here, and it is *why* these two shapes are what the structural engine's
// diagnostics must recommend for itself rather than pointing at the other engine.
mod self_callbacks {
    #[decycle::decycle(structural)]
    pub mod structural {
        #[decycle::decycle]
        pub trait Vis {
            fn with_ptr(&self, f: fn(&Self) -> i64) -> i64;
            fn with_dyn(&self, f: &dyn Fn(&Self) -> i64) -> i64;
        }
        pub struct A(pub i64, pub Box<B>);
        pub struct B(pub i64);

        impl Vis for A
        where
            B: Vis,
        {
            fn with_ptr(&self, f: fn(&Self) -> i64) -> i64 {
                f(self)
            }
            fn with_dyn(&self, f: &dyn Fn(&Self) -> i64) -> i64 {
                f(self)
            }
        }
        impl Vis for B
        where
            A: Vis,
        {
            fn with_ptr(&self, f: fn(&Self) -> i64) -> i64 {
                f(self)
            }
            fn with_dyn(&self, f: &dyn Fn(&Self) -> i64) -> i64 {
                f(self)
            }
        }
    }
}

// The rejection keys on what a bound MENTIONS, so `where Self: Sized` — which *bounds* `Self`
// rather than naming it in a bound, and is a very common method-level idiom — must keep working.
dual_mod! {
    method_where_self_sized {
        #[decycle]
        pub trait Vis {
            fn keep(&self) -> i64
            where
                Self: Sized;
        }
        pub struct A(pub i64, pub Box<B>);
        pub struct B(pub i64);

        impl Vis for A
        where
            B: Vis,
        {
            fn keep(&self) -> i64
            where
                Self: Sized,
            {
                self.0 + self.1.keep()
            }
        }
        impl Vis for B
        where
            A: Vis,
        {
            fn keep(&self) -> i64
            where
                Self: Sized,
            {
                self.0
            }
        }
    }
}

#[test]
fn a_method_level_where_self_sized_is_not_swept_up_by_the_rejection() {
    on_both!(method_where_self_sized, {
        assert_eq!(A(1, Box::new(B(2))).keep(), 3);
        assert_eq!(B(2).keep(), 2);
    });
}

#[test]
fn recommended_self_callback_shapes_work() {
    use self_callbacks::structural::*;
    assert_eq!(A(7, Box::new(B(0))).with_ptr(|a| a.0 * 2), 14);
    assert_eq!(B(7).with_ptr(|b| b.0 * 2), 14);
    let bump = 5;
    assert_eq!(A(7, Box::new(B(0))).with_dyn(&|a: &A| a.0 + bump), 12);
    assert_eq!(B(7).with_dyn(&|b: &B| b.0 + bump), 12);
}

// ===== 5. A method attribute macro is expanded EXACTLY ONCE =====
//
// The structural engine spliced the user's attrs verbatim onto both the natural and the terminator
// copy, so an attribute macro ran twice — contradicting `propagated_method_attrs`' contract and
// breaking outright for any macro that emits a named item (`E0407`) or that lints on duplication
// (`#[deprecated]` → two `useless_deprecated` errors). `attrcount::count_expansions` reports its own
// compile-time expansion ordinal at run time, so this counts rather than merely compiles.

use std::cell::RefCell;

thread_local! {
    static HITS: RefCell<Vec<(String, usize)>> = const { RefCell::new(Vec::new()) };
}

/// Called by code `attrcount::count_expansions` generates; see that crate's docs.
pub fn __decycle_attr_expansion_hit(label: &str, ordinal: usize) {
    HITS.with(|h| h.borrow_mut().push((label.to_string(), ordinal)));
}

fn drain_hits() -> Vec<(String, usize)> {
    HITS.with(|h| h.borrow_mut().drain(..).collect())
}

/// Each label is counted separately, so the three engines' modules cannot be written with
/// `dual_mod!` (it would give both copies the same label).
mod attr_expansions {
    #[decycle::decycle(structural)]
    pub mod structural {
        #[decycle::decycle]
        pub trait Ev {
            fn ev(&self) -> i64;
        }
        pub struct A(pub i64, pub Box<B>);
        pub struct B(pub i64);

        impl Ev for A
        where
            B: Ev,
        {
            #[attrcount::count_expansions(structural)]
            fn ev(&self) -> i64 {
                self.0 + self.1.ev()
            }
        }
        impl Ev for B
        where
            A: Ev,
        {
            fn ev(&self) -> i64 {
                self.0
            }
        }
    }

    #[decycle::decycle]
    pub mod ranked {
        #[decycle::decycle]
        pub trait Ev {
            fn ev(&self) -> i64;
        }
        pub struct A(pub i64, pub Box<B>);
        pub struct B(pub i64);

        impl Ev for A
        where
            B: Ev,
        {
            #[attrcount::count_expansions(ranked)]
            fn ev(&self) -> i64 {
                self.0 + self.1.ev()
            }
        }
        impl Ev for B
        where
            A: Ev,
        {
            fn ev(&self) -> i64 {
                self.0
            }
        }
    }

    /// The same code with no decycle attribute at all — the reference count.
    pub mod plain {
        pub trait Ev {
            fn ev(&self) -> i64;
        }
        pub struct A(pub i64, pub Box<B>);
        pub struct B(pub i64);

        impl Ev for A {
            #[attrcount::count_expansions(plain)]
            fn ev(&self) -> i64 {
                self.0 + self.1.ev()
            }
        }
        impl Ev for B {
            fn ev(&self) -> i64 {
                self.0
            }
        }
    }
}

#[test]
fn structural_expands_a_method_attribute_macro_exactly_once() {
    use attr_expansions::structural::{Ev, A, B};
    let _ = drain_hits();
    assert_eq!(A(1, Box::new(B(2))).ev(), 3);
    // One hit, and its ordinal is 1: the attribute was expanded once, full stop. Before the fix
    // this was `[("structural", 1), ("structural", 2)]` — the natural copy and the terminator copy.
    assert_eq!(drain_hits(), vec![("structural".to_string(), 1)]);
}

#[test]
fn ranked_and_plain_expand_a_method_attribute_macro_exactly_once_too() {
    {
        use attr_expansions::ranked::{Ev, A, B};
        let _ = drain_hits();
        assert_eq!(A(1, Box::new(B(2))).ev(), 3);
        assert_eq!(drain_hits(), vec![("ranked".to_string(), 1)]);
    }
    {
        use attr_expansions::plain::{Ev, A, B};
        let _ = drain_hits();
        assert_eq!(A(1, Box::new(B(2))).ev(), 3);
        assert_eq!(drain_hits(), vec![("plain".to_string(), 1)]);
    }
}
