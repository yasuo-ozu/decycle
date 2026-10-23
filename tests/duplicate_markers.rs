//! A `#[decycle]` marker is a flag: an item marked twice (as a macro that marks the items it emits
//! can leave one the user marked already) is marked once, in both engines. The ranked engine used to
//! take only the first marker and re-emit the second, which then failed as an unknown attribute
//! (or, with `decycle` imported into the module, announced the trait).

mod ext {
    // Announced, so that the ranked engine can carry it into a module through `#[decycle] use`.
    #[decycle::decycle]
    pub trait Ev {
        fn ev(&self, n: u32) -> u32;
    }
}

macro_rules! twice_marked {
    ($name:ident, $($engine:tt)*) => {
        #[decycle::decycle($($engine)*)]
        pub mod $name {
            #[decycle]
            #[decycle]
            pub trait Tr {
                fn f(&self, n: u32) -> u32;
            }

            #[decycle]
            #[decycle::decycle]
            use crate::ext::Ev;

            pub struct A;
            pub struct B;

            impl Tr for A
            where
                B: Tr,
            {
                fn f(&self, n: u32) -> u32 {
                    if n == 0 { 0 } else { B.f(n - 1) + 1 }
                }
            }

            impl Tr for B
            where
                A: Tr,
            {
                fn f(&self, n: u32) -> u32 {
                    if n == 0 { 0 } else { A.f(n - 1) + 2 }
                }
            }

            impl Ev for A
            where
                B: Ev,
            {
                fn ev(&self, n: u32) -> u32 {
                    if n == 0 { 0 } else { B.ev(n - 1) + 1 }
                }
            }

            impl Ev for B
            where
                A: Ev,
            {
                fn ev(&self, n: u32) -> u32 {
                    if n == 0 { 0 } else { A.ev(n - 1) + 2 }
                }
            }
        }
    };
}

twice_marked!(ranked,);
twice_marked!(structural, structural);

#[test]
fn a_twice_marked_trait_and_use_take_part_in_the_cycle_once() {
    use ext::Ev;
    {
        use ranked::Tr;
        assert_eq!(ranked::A.f(40), 60);
        assert_eq!(ranked::A.ev(40), 60);
    }
    {
        use structural::Tr;
        assert_eq!(structural::A.f(40), 60);
        assert_eq!(structural::A.ev(40), 60);
    }
}
