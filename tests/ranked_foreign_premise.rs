//! M2 regression: a cyclic-trait bound whose TARGET is a foreign, non-participant type
//! (`Vec<crate::other::Stmt>: Tr` — head `Vec`, no ranked impl to descend through) must survive
//! the ranked rewrite as an ordinary premise on the ORIGINAL trait, so the program compiles and
//! runs when the user supplies a matching impl of that original trait.
//!
//! Previously the two passes disagreed about what "cyclic" means: `remove_cyclic_bounds`
//! correctly KEPT the predicate (its target is no participant), but the where-clause rewrite
//! (`TraitReplacer`) still rank-lowered the `Tr` inside it to `TrRanked<Rank>` purely on the
//! trait's spelling — an unprovable obligation for a foreign head — and
//! `validate_impl_where_bounds` therefore had to abort such impls up front
//! ("this cyclic bound's target is not a type the ranked engine can rank-lower").
//!
//! The compile-fail side (NO matching impl supplied ⇒ plain `Vec<other::Stmt>: Tr` E0277s on
//! the original trait, never a `TrRanked` wall) is pinned by
//! `tests/ui/peel_foreign_last_segment.rs`.
use decycle::decycle;

pub mod other {
    pub struct Stmt;
}

// Default mode (`support_infinite_cycle` on): exercises the premise on the leaf and inductive
// impls, the Final delegating impl, AND the register-once fn inside `shadowing_module` (whose
// bare `Tr` spelling would otherwise resolve to the empty dummy shadow trait).
#[decycle]
mod m {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }

    pub struct Stmt(pub Option<Box<Stmt>>);

    impl Tr for Stmt
    where
        Vec<crate::other::Stmt>: Tr,
    {
        fn f(&self) -> usize {
            match &self.0 {
                Some(inner) => inner.f() + 1,
                None => 1,
            }
        }
    }
}

// The premise the impl above states, supplied by the user as an ordinary impl of the ORIGINAL
// trait (`Vec` is not a cycle participant; the ranked engine must not demand `TrRanked` of it).
impl m::Tr for Vec<other::Stmt> {
    fn f(&self) -> usize {
        40 + self.len()
    }
}

// Bounded mode: the same shape without the re-entry registry, so the premise is checked on the
// bounded rank ladder too.
#[decycle(recurse_level = 3, support_infinite_cycle = false)]
mod b {
    #[decycle]
    pub trait Tb {
        fn g(&self) -> usize;
    }

    pub struct Node(pub Option<Box<Node>>);

    impl Tb for Node
    where
        Vec<crate::other::Stmt>: Tb,
    {
        fn g(&self) -> usize {
            match &self.0 {
                Some(inner) => inner.g() + 1,
                None => 1,
            }
        }
    }
}

impl b::Tb for Vec<other::Stmt> {
    fn g(&self) -> usize {
        100 + self.len()
    }
}

#[test]
fn foreign_premise_survives_on_original_trait_unbounded_default() {
    use m::Tr;
    let s = m::Stmt(Some(Box::new(m::Stmt(None))));
    assert_eq!(s.f(), 2);
    // The premise impl is an ordinary, callable impl of the same (original) trait.
    let v: Vec<other::Stmt> = vec![other::Stmt];
    assert_eq!(v.f(), 41);
}

#[test]
fn foreign_premise_survives_on_original_trait_bounded() {
    use b::Tb;
    let n = b::Node(Some(Box::new(b::Node(Some(Box::new(b::Node(None)))))));
    assert_eq!(n.g(), 3);
    let v: Vec<other::Stmt> = vec![other::Stmt, other::Stmt];
    assert_eq!(v.g(), 102);
}
