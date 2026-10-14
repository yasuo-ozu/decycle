//! Traits are announced with `#[announce]` (re-exported as `decycle::announce`), not annotated with
//! `#[decycle]`: a trait of another crate that names a type of its crate by an absolute path,
//! one validated with `decycle::check`, and one with a type of its own module.

// `crate::tys::Unit` in a trait of another crate: absolute, so it resolves at the use site.
#[decycle::decycle]
mod m {
    use traitdef::tys;

    #[decycle]
    pub use traitdef::checked::FromTys;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub usize);

    impl FromTys for A
    where
        B: FromTys,
    {
        fn unit(&self, depth: usize) -> tys::Unit {
            match &self.0 {
                Some(b) => tys::Unit(b.unit(depth + 1).0 + 1),
                None => tys::Unit(depth),
            }
        }
    }

    impl FromTys for B {
        fn unit(&self, depth: usize) -> tys::Unit {
            tys::Unit(self.0 * 10 + depth)
        }
    }
}

#[test]
fn absolute_type_across_crates() {
    use traitdef::checked::FromTys as _;
    let a = m::A(Some(Box::new(m::B(3))));
    // B: 3 * 10 + 1 = 31; A: 31 + 1 = 32
    assert_eq!(a.unit(0).0, 32);
}

// A type of the trait's own module, spelled absolutely; `String` and `Vec<u8>` are std prelude
// names, accepted with `allow_std_prelude = true`.
mod same_crate_types {
    pub struct MyTy(pub usize);

    #[decycle::announce::announce(crate = ::decycle::announce, allow_std_prelude = true, decycle::check)]
    pub trait Produce {
        fn produce(&self) -> crate::same_crate_types::MyTy;
        fn name(&self, bytes: Vec<u8>) -> String;
    }

    impl Produce for u8 {
        fn produce(&self) -> MyTy {
            MyTy(*self as usize)
        }
        fn name(&self, bytes: Vec<u8>) -> String {
            format!("{}:{}", self, bytes.len())
        }
    }
}

#[test]
fn announced_trait_is_an_ordinary_trait() {
    use same_crate_types::Produce as _;
    assert_eq!(3u8.produce().0, 3);
    assert_eq!(3u8.name(vec![1, 2]), "3:2");
}

// `decycle::check(structural)` checks the rules of the structural engine, which accepts a default on
// a generic parameter (the ranked engine does not; see `tests/ui/check_default_param.rs`). A use site
// may leave the defaulted argument out.
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check(structural))]
pub trait Weigh<T = u8> {
    fn weigh(&self, t: T, depth: usize) -> usize;
}

#[decycle::decycle(structural)]
mod structural_default {
    #[decycle]
    use crate::Weigh;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Weigh for A
    where
        B: Weigh,
    {
        fn weigh(&self, t: u8, depth: usize) -> usize {
            self.0.as_ref().map_or(t as usize + depth, |b| b.weigh(t, depth + 1) + 1)
        }
    }

    impl Weigh<u8> for B
    where
        A: Weigh,
    {
        fn weigh(&self, t: u8, depth: usize) -> usize {
            self.0.as_ref().map_or(t as usize + depth, |a| a.weigh(t, depth + 1) + 100)
        }
    }
}

#[test]
fn structural_check_accepts_a_defaulted_parameter() {
    use crate::Weigh as _;
    let a = structural_default::A(Some(Box::new(structural_default::B(Some(Box::new(
        structural_default::A(None),
    ))))));
    // inner A at depth 2: 5 + 2 = 7; B: 7 + 100 = 107; outer A: 107 + 1 = 108
    assert_eq!(a.weigh(5, 0), 108);
}

// A trampoline reached under a name other than its trait's (`WalkMac` for `Walk`), given the trait's
// name by the `as` of the `#[decycle] use`: the module's impls spell `Walk`, and the trait itself is
// a plain `use` beside it (the two live in different namespaces).
mod renamed_trampoline {
    #[decycle::announce::announce(crate = ::decycle::announce, macro_name = WalkMac)]
    pub trait Walk {
        fn walk(&self, depth: usize) -> usize;
    }
}

#[decycle::decycle]
mod via_macro_name {
    use crate::renamed_trampoline::Walk;
    #[decycle]
    use crate::renamed_trampoline::WalkMac as Walk;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Walk for A
    where
        B: Walk,
    {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |b| b.walk(depth + 1))
        }
    }

    impl Walk for B
    where
        A: Walk,
    {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |a| a.walk(depth + 1) + 10)
        }
    }
}

#[test]
fn trampoline_renamed_with_macro_name() {
    use crate::renamed_trampoline::Walk as _;
    let a = via_macro_name::A(Some(Box::new(via_macro_name::B(Some(Box::new(
        via_macro_name::A(None),
    ))))));
    // inner A at depth 2: 2; B: 2 + 10 = 12; outer A: 12
    assert_eq!(a.walk(0), 12);
}

// `#[decycle]` on a trait outside a module announces it, as `#[announce]` would.
mod decycle_on_trait {
    #[decycle::decycle]
    pub trait Count {
        fn count(&self) -> usize;
    }
}

#[decycle::decycle]
mod uses_decycle_trait {
    #[decycle]
    pub use crate::decycle_on_trait::Count;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Count for A
    where
        B: Count,
    {
        fn count(&self) -> usize {
            self.0.as_ref().map_or(0, |b| b.count() + 1)
        }
    }

    impl Count for B
    where
        A: Count,
    {
        fn count(&self) -> usize {
            self.0.as_ref().map_or(0, |a| a.count() + 1)
        }
    }
}

#[test]
fn decycle_on_a_trait_announces_it() {
    use decycle_on_trait::Count as _;
    use uses_decycle_trait::{A, B};
    let a = A(Some(Box::new(B(Some(Box::new(A(None)))))));
    assert_eq!(a.count(), 2);
}
