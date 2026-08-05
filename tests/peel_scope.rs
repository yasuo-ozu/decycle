//! A cycle member is recognised only by a BARE ident. A multi-segment path is an outer type in
//! its entirety, even when its last segment happens to match a member's name.
#![allow(dead_code)]

use decycle::decycle;

pub mod other {
    pub struct Stmt(pub usize);
    pub struct Wrap<T>(pub T);
}

// `crate::other::Stmt` must NOT be peeled as though it were the local `Stmt`: peeling it emitted
// rank-lowering obligations against the FOREIGN type (a per-rank wall of
// `other::Stmt: TrRanked<((((..`). A genuine member nested inside a foreign wrapper is still
// found, because only the head path is rejected, not the recursion into its arguments.
#[decycle]
mod nested_member {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }
    pub struct Stmt(pub Option<Box<Stmt>>);

    // `crate::other::Wrap<Stmt>` is foreign at the head, but the BARE `Stmt` inside it is a real
    // member, so this peels to `Stmt: Tr` and compiles.
    impl Tr for Stmt
    where
        crate::other::Wrap<Stmt>: Tr,
    {
        fn f(&self) -> usize {
            match &self.0 {
                Some(inner) => inner.f() + 1,
                None => 0,
            }
        }
    }
}

#[test]
fn member_nested_in_a_foreign_wrapper_is_still_peeled() {
    use nested_member::Tr;
    assert_eq!(nested_member::Stmt(None).f(), 0);
    assert_eq!(
        nested_member::Stmt(Some(Box::new(nested_member::Stmt(None)))).f(),
        1
    );
}

// A `self::`-qualified member is the same item as the bare spelling — the convention
// `is_bare_cyclic_bound` already applies to the trait side — so it still peels.
#[decycle]
mod self_qualified_member {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }
    pub struct Stmt(pub Option<Box<Stmt>>);

    impl Tr for Stmt
    where
        Box<self::Stmt>: Tr,
    {
        fn f(&self) -> usize {
            match &self.0 {
                Some(inner) => inner.f() + 1,
                None => 0,
            }
        }
    }
}

#[test]
fn self_qualified_member_still_peels() {
    use self_qualified_member::Tr;
    assert_eq!(self_qualified_member::Stmt(None).f(), 0);
}
