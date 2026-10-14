//! A `#[decycle] use` that only names a trampoline (`WalkMac`) or a stand-in (`Size2`) is imported
//! under a trait's name but never used by name: the module's impls spell the trait through the plain
//! `use`. Under `#![deny(warnings)]` that must not be an `unused_imports` error, for either engine.
#![deny(warnings)]

pub mod d {
    #[decycle::announce::announce(crate = ::decycle::announce, macro_name = WalkMac)]
    pub trait Walk {
        fn walk(&self, depth: usize) -> usize;
    }
}

pub mod real {
    pub trait Size {
        fn size(&self, depth: usize) -> usize;
    }
}

pub mod stand {
    #[decycle::announce::announce(crate = ::decycle::announce, mock = crate::real::Size)]
    pub trait Size2 {
        fn size(&self, depth: usize) -> usize;
    }
}

macro_rules! cycle {
    ($attr:meta, $name:ident) => {
        #[$attr]
        pub mod $name {
            use crate::d::Walk;
            #[decycle]
            use crate::d::WalkMac as Walk;
            use crate::real::Size;
            #[decycle]
            use crate::stand::Size2 as Size;

            pub struct A(pub Option<Box<B>>);
            pub struct B(pub Option<Box<A>>);

            impl Walk for A
            where
                B: Size,
            {
                fn walk(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |b| b.size(depth + 1))
                }
            }

            impl Size for B
            where
                A: Walk,
            {
                fn size(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |a| a.walk(depth + 1) + 10)
                }
            }
        }
    };
}

cycle!(decycle::decycle, ranked);
cycle!(decycle::decycle(structural), structural);

#[test]
fn ranked_use_of_a_trampoline_under_deny_warnings() {
    use crate::d::Walk as _;
    let a = ranked::A(Some(Box::new(ranked::B(Some(Box::new(ranked::A(None)))))));
    // inner A at depth 2: 2; B: 2 + 10 = 12; outer A: 12
    assert_eq!(a.walk(0), 12);
}

#[test]
fn structural_use_of_a_trampoline_under_deny_warnings() {
    use crate::d::Walk as _;
    let a = structural::A(Some(Box::new(structural::B(Some(Box::new(
        structural::A(None),
    ))))));
    assert_eq!(a.walk(0), 12);
}
