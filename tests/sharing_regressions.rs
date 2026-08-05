//! Regressions for the ranked engine's premise-sharing pass, which unions the where-clause
//! premises of the impls in a cycle so every rank-lowered copy carries what it needs. Sharing too
//! much rejects code that is valid without the macro.
#![allow(dead_code)]

use decycle::decycle;

// A sibling's predicate that mentions a LIFETIME the receiving impl does not declare must not be
// shared into it. `impl_param_idents` used to report only type/const params, so the "declares
// everything it mentions" guard never looked for lifetimes and injected `&'a str: Clone` into
// `B`'s copies -> `error[E0261]: use of undeclared lifetime name 'a`, on code that compiles fine
// without `#[decycle]`.
#[decycle]
mod sibling_lifetime_pred {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }
    pub struct A(pub Box<B>);
    pub struct B(pub usize);
    // `'a` appears only in a co-bound; `B`'s impl declares no lifetime at all.
    impl<'a> Tr for A
    where
        B: Tr,
        &'a str: Clone,
    {
        fn f(&self) -> usize {
            self.0.f() + 1
        }
    }
    impl Tr for B
    where
        A: Tr,
    {
        fn f(&self) -> usize {
            self.0
        }
    }
}

#[test]
fn sibling_lifetime_predicate_is_not_shared() {
    use sibling_lifetime_pred::Tr;
    assert_eq!(sibling_lifetime_pred::B(7).f(), 7);
    assert_eq!(
        sibling_lifetime_pred::A(Box::new(sibling_lifetime_pred::B(7))).f(),
        8
    );
}

// The counterweight: `'static` is always in scope, so it must NOT count as a parameter the
// receiving impl has to declare — otherwise the guard above would withhold every ordinary
// `T: 'static` co-bound and the pass would under-share instead of over-sharing.
#[decycle]
mod static_cobound_still_shared {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }
    pub struct A<T>(pub T, pub Box<B>);
    pub struct B(pub usize);
    impl<T> Tr for A<T>
    where
        B: Tr,
        T: Clone + 'static,
    {
        fn f(&self) -> usize {
            self.1.f() + 1
        }
    }
    impl Tr for B
    where
        A<u8>: Tr,
    {
        fn f(&self) -> usize {
            self.0
        }
    }
}

#[test]
fn static_cobound_is_still_shared() {
    use static_cobound_still_shared::Tr;
    assert_eq!(static_cobound_still_shared::B(4).f(), 4);
    assert_eq!(
        static_cobound_still_shared::A(1u8, Box::new(static_cobound_still_shared::B(4))).f(),
        5
    );
}

// Grouping used to be "every adopted impl of the trait", keyed by trait ident, so an impl with no
// obligation path to the cycle still inherited its premises: `C<T>` acquired `T: Clone` from
// `A<T>`, and the failure only surfaced at a caller, with the diagnostic pointing at `A`'s impl.
// Sharing is now scoped to the impls each impl's rank chain can actually reach.
pub struct NoClone;

#[decycle]
mod acyclic_sibling {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }
    pub struct A<T>(pub T, pub Box<B>);
    pub struct B(pub Option<Box<A<i32>>>);
    pub struct C<T>(pub T); // NOT part of the A <-> B cycle

    impl<T> Tr for A<T>
    where
        B: Tr,
        T: Clone,
    {
        fn f(&self) -> usize {
            self.1.f()
        }
    }
    impl Tr for B
    where
        A<i32>: Tr,
    {
        fn f(&self) -> usize {
            self.0.as_ref().map(|a| a.f()).unwrap_or(1) + 1
        }
    }
    // No premises of its own, and none may be foisted on it.
    impl<T> Tr for C<T> {
        fn f(&self) -> usize {
            7
        }
    }
}

#[test]
fn acyclic_impl_does_not_inherit_cycle_premises() {
    use acyclic_sibling::Tr;
    // `NoClone` is deliberately not `Clone`: this call does not compile if `C<T>` inherited
    // `T: Clone` from the A/B cycle.
    assert_eq!(acyclic_sibling::C(NoClone).f(), 7);
    assert_eq!(acyclic_sibling::B(None).f(), 2);
}

// Two DISJOINT cycles of one trait must not union each other's premises either.
#[decycle]
mod disjoint_cycles {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }
    pub struct A<T>(pub T, pub Box<B>);
    pub struct B(pub usize);
    pub struct P<T>(pub T, pub Box<Q>);
    pub struct Q(pub usize);

    impl<T> Tr for A<T>
    where
        B: Tr,
        T: Clone,
    {
        fn f(&self) -> usize {
            self.1.f()
        }
    }
    impl Tr for B
    where
        A<i32>: Tr,
    {
        fn f(&self) -> usize {
            self.0
        }
    }
    // A separate cycle whose participant is NOT `Clone`-bounded.
    impl<T> Tr for P<T>
    where
        Q: Tr,
    {
        fn f(&self) -> usize {
            self.1.f() + 1
        }
    }
    impl Tr for Q
    where
        P<crate::NoClone>: Tr,
    {
        fn f(&self) -> usize {
            self.0
        }
    }
}

#[test]
fn disjoint_cycles_do_not_share_premises() {
    use disjoint_cycles::Tr;
    assert_eq!(disjoint_cycles::B(3).f(), 3);
    // `P<NoClone>` only compiles if the A/B cycle's `T: Clone` stayed out of P/Q.
    assert_eq!(
        disjoint_cycles::P(NoClone, Box::new(disjoint_cycles::Q(4))).f(),
        5
    );
}
