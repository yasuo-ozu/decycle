//! Foreign `#[decycle]` traits whose default bodies hold `$crate` / `$(..)*` macros, used in a
//! `#[decycle] mod` of another crate (AUDIT decycle #1, #2), and a `mock = path` stand-in's
//! trampoline reachable at the item's path in another crate (formerly `alter_macro_name`, AUDIT
//! decycle #5).

#[decycle::decycle]
mod m {
    #[decycle]
    pub use traitdef::DollarCrateDefault;
    #[decycle]
    pub use traitdef::RepetitionDefault;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub usize);

    impl DollarCrateDefault for A
    where
        B: DollarCrateDefault,
    {
        fn base(&self) -> usize {
            self.0.as_ref().map_or(0, |b| b.with_helper())
        }
    }
    impl DollarCrateDefault for B {
        fn base(&self) -> usize {
            self.0
        }
    }

    impl RepetitionDefault for A
    where
        B: RepetitionDefault,
    {
        fn base2(&self) -> usize {
            macro_rules! first {
                ($($x:expr),*) => { [$($x),*][0] };
            }
            first!(self.0.as_ref().map_or(0, |b| b.summed()), 99)
        }
    }
    impl RepetitionDefault for B {
        fn base2(&self) -> usize {
            self.0
        }
    }
}

#[test]
fn dollar_crate_in_default_body() {
    use traitdef::DollarCrateDefault as _;
    let a = m::A(Some(Box::new(m::B(1))));
    // B: 1 + 7 = 8; A: base 8 + 7 = 15
    assert_eq!(a.with_helper(), 15);
}

#[test]
fn repetition_in_default_bodies_and_user_code() {
    use traitdef::RepetitionDefault as _;
    let a = m::A(Some(Box::new(m::B(1))));
    // B: 1 + 3 = 4; A: base 4 + 3 = 7
    assert_eq!(a.summed(), 7);
}

macro_rules! count_entries {
    (@announce_format "1" [$($list:tt)*] $($key:ident)::+ ! => [$($meta:tt)*] { $($item:tt)* }) => {
        const STAND_IN: &str = stringify!($($meta)* => $($item)*);
    };
}

// A stand-in's carrier resolves at the stand-in's path in the defining crate.
traitdef::stand::Weigh! { [traitdef::stand::Weigh!, count_entries!] }

#[test]
fn stand_in_carrier_is_reachable_at_its_path() {
    assert!(STAND_IN.contains("mock"), "{}", STAND_IN);
    // The default body travels with the copy.
    assert!(STAND_IN.contains("fn doubled"), "{}", STAND_IN);
}
