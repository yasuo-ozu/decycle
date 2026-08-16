//! Regressions for `#[decycle]` applied to a **standalone trait** (`ranked::process_trait`), the
//! form a macro crate uses to define a trait once and let any `#[decycle] mod` route it.

// The `marker` path is spliced into the trait definition that the generated carrier macro re-emits,
// so a `crate::`-rooted marker means "the crate the macro is EXPANDED in" — which is why the docs
// ask for a marker path that is "practically absolute and accessible from anywhere". Both tests
// below define and use the trait in one crate, where the two coincide; clippy's warning is still a
// fair description of what such a path would mean across a crate boundary.
#![allow(clippy::crate_in_macro_def)]

/// **Prelude names must be usable in a standalone `#[decycle]` trait.**
///
/// `process_trait` interns the trait's non-absolute type paths so the definition survives being
/// re-quoted through the generated carrier macro, and the allowed-path set was `allow_crate()` +
/// `allow_primitive()` — no prelude. type-leak then hard-errors on any relative trait path, which
/// `process_trait` turned into a bare `use absolute path` naming neither decycle nor the escape
/// hatch. So `fn f<T: Clone>(..)` was rejected outright on a standalone trait, while the identical
/// trait written inside a `#[decycle] mod` compiled fine.
///
/// Still a limitation, and why `Into<usize>` below is spelled absolutely: type-leak matches an
/// allowed path segment-by-segment INCLUDING its generic arguments, so an argument-less entry
/// (`Into`) never matches an argument-carrying use (`Into<usize>`). Prelude traits are therefore
/// usable bare (`Clone`, `Send`, `Default`, …) but not with arguments; that comparison lives in
/// type-leak's `visit_trait_bound`, not here. Prelude *types* (`Vec<u8>`, `Option<usize>`) go
/// through a different check and are fine either way.
mod prelude_bounds {
    /// The interning anchor. `Vec<u8>` / `Option<usize>` are non-absolute type paths, which the
    /// documented contract says need a `marker` — see the `interning` module below for what the
    /// marker now actually does.
    pub struct Marker;

    #[decycle::decycle(marker = crate::prelude_bounds::Marker)]
    pub trait Tally {
        fn tally<T: Clone + ::core::convert::Into<usize>>(&self, seed: T) -> usize;
        fn bag(&self, v: Vec<u8>) -> Option<usize>;
    }

    #[decycle::decycle]
    pub mod m {
        #[decycle]
        pub use super::Tally;

        pub struct A(pub Box<B>);
        pub struct B(pub Option<Box<A>>);

        impl Tally for A
        where
            B: Tally,
        {
            fn tally<T: Clone + ::core::convert::Into<usize>>(&self, seed: T) -> usize {
                seed.clone().into() + self.0.tally(seed)
            }
            fn bag(&self, v: Vec<u8>) -> Option<usize> {
                Some(v.len())
            }
        }

        impl Tally for B
        where
            A: Tally,
        {
            fn tally<T: Clone + ::core::convert::Into<usize>>(&self, seed: T) -> usize {
                match &self.0 {
                    Some(a) => a.tally(seed),
                    None => 0,
                }
            }
            fn bag(&self, v: Vec<u8>) -> Option<usize> {
                v.first().map(|b| *b as usize)
            }
        }
    }

    #[test]
    fn prelude_trait_bounds_are_accepted() {
        use m::Tally as _;
        let one = m::A(Box::new(m::B(None)));
        assert_eq!(one.tally(3u8), 3);
        let two = m::A(Box::new(m::B(Some(Box::new(m::A(Box::new(m::B(None))))))));
        assert_eq!(two.tally(3u8), 6);
        assert_eq!(one.bag(vec![1, 2, 3]), Some(3));
        assert_eq!(m::B(None).bag(vec![9, 8]), Some(9));
    }
}

/// **The `marker` / type-interning path was entirely dead.**
///
/// `Leaker::reduce_roots()` was never called anywhere in the crate, and it is the only thing that
/// populates `reachable_types` — the set `finish()` reads. So `finish()` always returned an empty
/// referrer: nothing was interned, no `Repeater` impls were emitted, and `marker` was accepted and
/// silently ignored (the `specify 'marker' arg` abort was unreachable). On top of that the carrier
/// macro's token stream was materialised BEFORE the interning rewrite, so even a populated referrer
/// would have shipped the un-rewritten definition.
///
/// The visible consequence: a trait naming a type relative to its OWN definition site travelled
/// through the carrier macro verbatim, and at the use site that name does not exist —
/// `error[E0412]: cannot find type MyTy in this scope`. Interning replaces it with
/// `<Marker as Repeater<…>>::Type`, which names the same type from anywhere.
mod interning {
    pub struct MyTy(pub usize);
    pub struct Marker;

    #[decycle::decycle(marker = crate::interning::Marker)]
    pub trait Produce {
        /// `MyTy` is spelled relative to THIS module — the shape that needs interning.
        fn produce(&self, depth: u32) -> MyTy;
    }

    #[decycle::decycle]
    pub mod m {
        #[decycle]
        pub use super::Produce;

        // `MyTy` is deliberately NOT imported here: the carried trait definition has to name it
        // without help from this module's scope.
        pub struct A(pub Box<B>);
        pub struct B(pub Option<Box<A>>);

        impl Produce for A
        where
            B: Produce,
        {
            fn produce(&self, depth: u32) -> crate::interning::MyTy {
                if depth == 0 {
                    return crate::interning::MyTy(1);
                }
                crate::interning::MyTy(self.0.produce(depth - 1).0 + 1)
            }
        }

        impl Produce for B
        where
            A: Produce,
        {
            fn produce(&self, depth: u32) -> crate::interning::MyTy {
                match &self.0 {
                    Some(a) => crate::interning::MyTy(a.produce(depth).0 * 2),
                    None => crate::interning::MyTy(10),
                }
            }
        }
    }

    #[test]
    fn a_relative_type_in_a_carried_trait_is_interned() {
        use m::Produce as _;
        let a = m::A(Box::new(m::B(None)));
        assert_eq!(a.produce(0).0, 1);
        assert_eq!(a.produce(1).0, 11);
        assert_eq!(m::B(Some(Box::new(m::A(Box::new(m::B(None)))))).produce(0).0, 2);
    }
}
