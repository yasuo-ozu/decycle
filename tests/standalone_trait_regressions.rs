//! A trait defined outside any `#[decycle]` module is announced with `#[announce]` (re-exported as
//! `decycle::announce`) — the form a macro crate uses to define a trait once and let any
//! `#[decycle] mod` route it with `#[decycle] use`.

/// **Prelude names must be usable in an announced trait.**
///
/// announce accepts the bare std prelude names (`Clone`, `Vec<u8>`, `Option<usize>`, ..) only with
/// `allow_std_prelude = true`; without it a bare `Vec<u8>` is a path relative to the defining
/// module and is rejected at the definition (see `tests/ui/announce_relative_path.rs`). Traits
/// with arguments (`Into<usize>`) are spelled absolutely, as announce requires of every trait path.
mod prelude_bounds {
    #[decycle::announce::announce(
        crate = ::decycle::announce,
        allow_std_prelude = true,
        decycle::check
    )]
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

/// **A type of the trait's own module travels as an absolute path.**
///
/// The trait is re-quoted into whatever module routes it, where a name relative to its definition
/// does not exist (`error[E0412]: cannot find type MyTy in this scope`). announce rejects such a
/// name at the definition (`tests/ui/announce_relative_path.rs`); `crate::absolute_types::MyTy`
/// names the same type from anywhere in the crate, and `$crate::..` from any crate.
mod absolute_types {
    pub struct MyTy(pub usize);

    #[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
    pub trait Produce {
        /// `crate::absolute_types::MyTy` is the absolute spelling of a type of this module.
        fn produce(&self, depth: u32) -> crate::absolute_types::MyTy;
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
            fn produce(&self, depth: u32) -> crate::absolute_types::MyTy {
                if depth == 0 {
                    return crate::absolute_types::MyTy(1);
                }
                crate::absolute_types::MyTy(self.0.produce(depth - 1).0 + 1)
            }
        }

        impl Produce for B
        where
            A: Produce,
        {
            fn produce(&self, depth: u32) -> crate::absolute_types::MyTy {
                match &self.0 {
                    Some(a) => crate::absolute_types::MyTy(a.produce(depth).0 * 2),
                    None => crate::absolute_types::MyTy(10),
                }
            }
        }
    }

    #[test]
    fn an_absolute_type_in_a_carried_trait_resolves() {
        use m::Produce as _;
        let a = m::A(Box::new(m::B(None)));
        assert_eq!(a.produce(0).0, 1);
        assert_eq!(a.produce(1).0, 11);
        assert_eq!(m::B(Some(Box::new(m::A(Box::new(m::B(None)))))).produce(0).0, 2);
    }
}
