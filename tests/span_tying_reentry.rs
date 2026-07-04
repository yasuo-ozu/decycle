//! C3 acceptance test: atom↔span `A: Spanned<Span = Self::Sp>`
//! (`docs/decycle-integration/impl-decycle-ranking-and-span.md`, C3 section, in
//! `/home/yasuo/ghq/github.com/yasuo-ozu/syan2`).
//!
//! syan's real `#[recurse]` surfaces the span type through a `Self`-projected associated type
//! (`Self::__SpanParam`) in a METHOD bound, so the span type is never a free type param on the
//! re-entry surface (which only declares `DclSelf` + the trait/method tycons — a bare `S` would
//! be E0433-unnameable there). Before this fix, `emit_reentry_items`'s `m_where` copied the
//! method where-clause into the free `#Re` re-entry fn VERBATIM: `normalize_reentry_sig` only
//! bare-`Self`-substitutes the signature's params/output (`replace_self`, single-segment `Self`
//! only), never the where-clause, so a `Self::Sp` PROJECTION (a 2-segment path) survived
//! literally into a fn that has no `Self` in scope — E0411 "cannot find type `Self`". This test
//! reproduces exactly that shape (a `#[decycle]` trait with an associated type `Sp`, and a
//! method generic `At` bounded by `Spanned<Span = Self::Sp>`) inside a genuine two-trait cycle,
//! and asserts it compiles and runs UNBOUNDED past `recurse_level`.
//!
//! Per the doc's C3.2 caveat: keeping the span bound in the method WHERE-CLAUSE (rather than
//! inline on the generic parameter) is syan's actual, recommended invariant — a type alias
//! `where`-clause is weakly enforced on stable Rust, and the fn-pointer alias body here names no
//! projection at all (`alias_needs_bound` is false), so C3.1 (the `m_where` `SelfSubst`) alone
//! closes this case; C3.2 (the alias where-clause's method-generic-projection extension) is
//! implemented as specified but is belt-and-suspenders for this particular surface.
//!
//! `Spanned` is an ordinary trait, NOT `#[decycle]`-listed — it must stay a leaf bound outside
//! `replacing_table`/`TraitReplacer` so its `Span =` associated-type equality is never rewritten
//! by the ranking machinery (which is what avoids an `E0271` rank disagreement).

#![allow(dead_code)]

use decycle::decycle;

/// A tiny, self-contained stand-in for syan's `Spanned` leaf trait. Deliberately NOT
/// `#[decycle]`-listed: it must remain an ordinary, unranked leaf bound.
pub trait Spanned {
    type Span;
    fn span(&self) -> Self::Span;
}

/// Two concrete (atom, span) instantiations, interleaved past the floor — proves the re-entry
/// key (which folds `Self` + trait/method type args, NOT the where-clause-only `Sp`) still
/// separates them correctly (per the doc's "fail-closed" note: `S` never needs to be in the
/// key, and none is added).
pub struct Sp1(pub u32);
pub struct Sp2(pub u64);
pub struct Atom1;
pub struct Atom2;
impl Spanned for Atom1 {
    type Span = Sp1;
    fn span(&self) -> Sp1 {
        Sp1(1)
    }
}
impl Spanned for Atom2 {
    type Span = Sp2;
    fn span(&self) -> Sp2 {
        Sp2(2)
    }
}

#[decycle(recurse_level = 2)]
mod span_tying_m {
    // The span-tying method (`m`) is never itself on the recursive path — matching
    // `assoc_carrying_m` in `tests/unbounded_reentry.rs` — but `emit_reentry_items` still
    // unconditionally builds its `__Mk`/`__Fp`/`__Re` machinery for EVERY method of every
    // `#[decycle]` trait, so this alone already exercises the defect at COMPILE time.
    #[decycle]
    pub trait Pr {
        type Sp;
        fn m<At>(&self, a: At, n: usize) -> usize
        where
            At: crate::Spanned<Span = Self::Sp>;
        fn step(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct Node;
    pub struct Other;
    impl Pr for Node
    where
        Other: Cb,
    {
        type Sp = crate::Sp1;
        fn m<At>(&self, a: At, n: usize) -> usize
        where
            At: crate::Spanned<Span = Self::Sp>,
        {
            // Actually uses the atom's span, proving the bound is real (not vestigial).
            let _sp: crate::Sp1 = a.span();
            n
        }
        fn step(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                Other.cb(n - 1) + 1
            }
        }
    }
    impl Cb for Other
    where
        Node: Pr,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                Node.step(n - 1) + 1
            }
        }
    }
}

// A second cycle with a DIFFERENT `Sp` assignment (`Sp2`), so both `(atom, span)`
// instantiations exist in the same binary — regresses the "no `S` in the key" fail-closed claim.
#[decycle(recurse_level = 2)]
mod span_tying_m2 {
    #[decycle]
    pub trait Pr {
        type Sp;
        fn m<At>(&self, a: At, n: usize) -> usize
        where
            At: crate::Spanned<Span = Self::Sp>;
        fn step(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct Node;
    pub struct Other;
    impl Pr for Node
    where
        Other: Cb,
    {
        type Sp = crate::Sp2;
        fn m<At>(&self, a: At, n: usize) -> usize
        where
            At: crate::Spanned<Span = Self::Sp>,
        {
            let _sp: crate::Sp2 = a.span();
            n
        }
        fn step(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                Other.cb(n - 1) + 1
            }
        }
    }
    impl Cb for Other
    where
        Node: Pr,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                Node.step(n - 1) + 1
            }
        }
    }
}

#[test]
fn span_tying_reentry_compiles_and_runs_unbounded() {
    use span_tying_m::Pr;
    // Plain-typed recursion, driven well past the `recurse_level = 2` floor.
    assert_eq!(span_tying_m::Node.step(2000), 2000);
    // The span-tying method, called from outside — `Self::Sp` resolved to a concrete `Sp1` at
    // an already-known `Self`, no generic rank involved.
    assert_eq!(span_tying_m::Node.m(Atom1, 5), 5);
}

#[test]
fn span_tying_reentry_second_instantiation_is_isolated() {
    use span_tying_m2::Pr;
    assert_eq!(span_tying_m2::Node.step(2000), 2000);
    assert_eq!(span_tying_m2::Node.m(Atom2, 7), 7);
}
