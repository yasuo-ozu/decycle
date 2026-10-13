//! `#[decycle(mock = path)] trait Stand` declares a stand-in for the real trait at `path`: only
//! the trampoline is emitted (at the stand-in's path); `#[decycle] use` sites receive the copy as
//! the real trait, so the module's impls implement the REAL trait and their cycles are resolved
//! as for any `#[decycle]` trait. Plain `#[announce(mock = path)]` copies (announce-std) work the
//! same way.

// A stand-in named like its real trait (another crate's).
#[decycle::decycle]
mod m {
    #[decycle]
    use traitdef::stand::Weigh;
    use traitdef::real::Weigh;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Weigh for A
    where
        B: Weigh,
    {
        fn weigh(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |b| b.weigh(depth + 1) + 1)
        }
    }

    impl Weigh for B
    where
        A: Weigh,
    {
        fn weigh(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth * 10, |a| a.weigh(depth + 1) + 100)
        }
    }
}

#[test]
fn stand_in_impls_implement_the_real_trait() {
    // `real::Weigh` is the only trait in scope: the impls are of the real trait.
    use traitdef::real::Weigh;
    let a = m::A(Some(Box::new(m::B(Some(Box::new(m::A(None)))))));
    // inner A at depth 2: 2; B: 2 + 100 = 102; outer A: 102 + 1 = 103
    assert_eq!(a.weigh(0), 103);
    // The real trait's default method.
    assert_eq!(m::B(None).doubled(), 0);
    fn takes_real<T: traitdef::real::Weigh>(t: &T) -> usize {
        t.weigh(1)
    }
    assert_eq!(takes_real(&m::B(None)), 10);
}

// A stand-in with its own name, `#[decycle] use`d under the real trait's name.
#[decycle::decycle]
mod renamed {
    use traitdef::real_unit::Measure;
    use traitdef::tys;

    #[decycle]
    use traitdef::stand_renamed::MeasureStandIn as Measure;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Measure for A
    where
        B: Measure,
    {
        fn measure(&self, depth: usize) -> tys::Unit {
            match &self.0 {
                Some(b) => tys::Unit(b.measure(depth + 1).0 + 1),
                None => tys::Unit(depth),
            }
        }
    }

    impl Measure for B
    where
        A: Measure,
    {
        fn measure(&self, depth: usize) -> tys::Unit {
            match &self.0 {
                Some(a) => tys::Unit(a.measure(depth + 1).0 + 10),
                None => tys::Unit(depth),
            }
        }
    }
}

#[test]
fn renamed_stand_in() {
    use traitdef::real_unit::Measure;
    let a = renamed::A(Some(Box::new(renamed::B(Some(Box::new(renamed::A(None)))))));
    assert_eq!(a.measure(0).0, 13);
}

// The stand-in's macro is reachable at the item's path, and its entry carries `mock(path)`.
macro_rules! show {
    ([$($list:tt)*] $($key:ident)::+ ! => [$($meta:tt)*] { $($item:tt)* }) => {
        const META: &str = stringify!($($meta)*);
        const ITEM: &str = stringify!($($item)*);
    };
}
traitdef::stand::Weigh! { [traitdef::stand::Weigh!, show!] }

#[test]
fn stand_in_macro_carries_mock_meta() {
    assert!(META.contains("mock") && META.contains("real :: Weigh"), "{}", META);
    assert!(ITEM.contains("trait Weigh"), "{}", ITEM);
}

// A stand-in in the same crate: `crate::` reaches the consumer as `$crate::`.
mod local {
    pub trait Count {
        fn count(&self, depth: usize) -> usize;
    }
    pub mod stand {
        #[decycle::decycle(mock = crate::local::Count)]
        pub trait Count {
            fn count(&self, depth: usize) -> usize;
        }
    }
}

#[decycle::decycle]
mod same_crate {
    use crate::local::Count;
    #[decycle]
    use crate::local::stand::Count;

    pub struct A(pub Option<Box<A>>);

    impl Count for A
    where
        A: Count,
    {
        fn count(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |a| a.count(depth + 1))
        }
    }
}

#[test]
fn same_crate_stand_in() {
    use local::Count;
    let a = same_crate::A(Some(Box::new(same_crate::A(Some(Box::new(same_crate::A(None)))))));
    assert_eq!(a.count(0), 2);
}

// A plain `#[announce(mock = ::core::fmt::Display)]` trait (announce-std) with `#[decycle] use`.
#[decycle::decycle]
mod std_mock {
    #[decycle]
    use announce_std::fmt::Display;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Display for A
    where
        B: Display,
    {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            match &self.0 {
                Some(b) => write!(f, "A({})", b),
                None => write!(f, "A"),
            }
        }
    }

    impl Display for B
    where
        A: Display,
    {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            match &self.0 {
                Some(a) => write!(f, "B({})", a),
                None => write!(f, "B"),
            }
        }
    }
}

#[test]
fn announce_std_mock() {
    let a = std_mock::A(Some(Box::new(std_mock::B(Some(Box::new(std_mock::A(None)))))));
    assert_eq!(a.to_string(), "A(B(A))");
}
