//! M9: `Foreign: super::x::Tr` — the documented opt-out premise binding a foreign type to the
//! ORIGINAL, un-ranked trait — must survive re-emission of the adopted impls two modules deeper.
//!
//! `LiftRelative` refused to lift any path whose LAST segment named a routed trait, so the
//! `super::`-spelled premise (and the matching qualified call in the body) was left behind where
//! `super::x` no longer resolves: E0433 "could not find `x` in `super`", while the `crate::`
//! spelling of the identical program compiled. Only the bare / `self::`-qualified spelling is the
//! cycle-edge signal that must stay un-lifted; a longer-qualified reference is an ordinary
//! premise. The lifted alias must also stay OUT of method-resolution scope: a flat
//! `use super::x::Tr as _Alias;` in the module made `self.0.f()` ambiguous (E0034) between the
//! ranked twin and the original trait, which is why the aliases live behind `__DecycleRelMod_*`.
use decycle::decycle;

pub mod x {
    #[decycle::decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }
}

impl x::Tr for String {
    fn f(&self) -> usize {
        9
    }
}

#[decycle]
mod m {
    #[decycle]
    pub use crate::x::Tr;

    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl Tr for A
    where
        B: Tr,
        String: super::x::Tr,
    {
        fn f(&self) -> usize {
            self.0.f() + <String as super::x::Tr>::f(&String::new())
        }
    }

    impl Tr for B
    where
        A: Tr,
    {
        fn f(&self) -> usize {
            self.0.as_ref().map(|a| a.f()).unwrap_or(1)
        }
    }
}

#[test]
fn super_qualified_premise_and_call() {
    use x::Tr as _;
    assert_eq!(m::A(Box::new(m::B(None))).f(), 10);
    assert_eq!(m::B(Some(Box::new(m::A(Box::new(m::B(None)))))).f(), 10);
}
