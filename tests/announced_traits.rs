//! Traits are announced with `#[announce]` (re-exported as `decycle::announce`), not annotated with
//! `#[decycle]`: a trait of another crate that names a type of its crate by an absolute path,
//! one validated with `decycle::check`, one with a type of its own module, and a private stand-in.

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

// A private stand-in (`mock = path`) is announced at the item's visibility: its trampoline is
// usable at its path within the crate, and the stand-in itself is not emitted as a trait.
mod private_stand {
    #[allow(dead_code)]
    pub trait Real {
        fn real(&self) -> u8;
    }
    pub(crate) mod stand {
        #[decycle::announce::announce(crate = ::decycle::announce, mock = crate::private_stand::Real)]
        pub(crate) trait Real {
            fn real(&self) -> u8;
        }
    }
}

macro_rules! private_stand_show {
    ([$($list:tt)*] $($key:ident)::+ ! => [$($meta:tt)*] { $($item:tt)* }) => {
        const PRIVATE_STAND: &str = stringify!($($meta)* => $($item)*);
    };
}
private_stand::stand::Real! { [private_stand::stand::Real!, private_stand_show!] }

#[test]
fn private_stand_in_macro_at_item_path() {
    assert!(PRIVATE_STAND.contains("mock"), "{}", PRIVATE_STAND);
    assert!(PRIVATE_STAND.contains("trait Real"), "{}", PRIVATE_STAND);
}
