//! `#[decycle]` traits are announced in announce's module-wrapper form, so decycle's own path
//! handling (`allowed_paths`, a relative `marker`) is not second-guessed by announce
//! (AUDIT part 3, decycle #1-#3).

// `#[decycle(allowed_paths = [tys])]` on a trait of another crate.
#[decycle::decycle]
mod m {
    use traitdef::tys;

    #[decycle]
    pub use traitdef::allowed::FromTys;

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
fn allowed_paths_across_crates() {
    use traitdef::allowed::FromTys as _;
    let a = m::A(Some(Box::new(m::B(3))));
    // B: 3 * 10 + 1 = 31; A: 31 + 1 = 32
    assert_eq!(a.unit(0).0, 32);
}

// The doc example's form: a relative `marker`, and a relative type that needs interning.
mod relative_marker {
    pub struct MyTy(pub usize);
    #[allow(dead_code)]
    pub struct Marker;

    #[decycle::decycle(marker = Marker)]
    pub trait Produce {
        fn produce(&self) -> MyTy;
    }

    impl Produce for u8 {
        fn produce(&self) -> MyTy {
            MyTy(*self as usize)
        }
    }
}

#[test]
fn relative_marker() {
    use relative_marker::Produce as _;
    assert_eq!(3u8.produce().0, 3);
}

// A private stand-in (`mock = path`) is announced at the item's visibility: its trampoline is
// usable at its path within the crate, and the stand-in itself is not emitted as a trait.
mod private_stand {
    #[allow(dead_code)]
    pub trait Real {
        fn real(&self) -> u8;
    }
    pub(crate) mod stand {
        #[decycle::decycle(mock = crate::private_stand::Real)]
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
