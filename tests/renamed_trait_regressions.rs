//! `#[decycle] use path::T as R;` renames the carried trait to `R` once. It used to be renamed twice,
//! the second time by matching the ORIGINAL name against every trait of the module, which hit the
//! wrong trait whenever the original name was also the name of another trait there:
//!
//! - a trait of the module named like the original (`T`), and
//! - a chain of renames, where one `use`'s alias is another's original (`X as Y; Y as Z`).
//!
//! Both worked in 0.5; both are pinned for the ranked and the structural engine.
#![allow(dead_code)]

pub mod d {
    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub trait Walk {
        fn walk(&self, depth: usize) -> usize;
    }

    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub trait X {
        fn x(&self, depth: usize) -> usize;
    }

    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub trait Y {
        fn y(&self, depth: usize) -> usize;
    }
}

// A local `Walk` beside `d::Walk` used as `W`.
macro_rules! local_name_is_the_original {
    ($attr:meta, $name:ident) => {
        #[$attr]
        mod $name {
            #[decycle]
            use crate::d::Walk as W;

            #[decycle]
            pub trait Walk {
                fn w2(&self) -> usize;
            }

            pub struct A(pub Option<Box<B>>);
            pub struct B(pub Option<Box<A>>);

            impl W for A
            where
                B: Walk,
            {
                fn walk(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |b| b.w2() + depth)
                }
            }

            impl Walk for B
            where
                A: W,
            {
                fn w2(&self) -> usize {
                    self.0.as_ref().map_or(1, |a| a.walk(1) + 10)
                }
            }
        }
    };
}

local_name_is_the_original!(decycle::decycle, ranked_local);
local_name_is_the_original!(decycle::decycle(structural), structural_local);

#[test]
fn ranked_local_trait_named_like_the_original() {
    use crate::d::Walk as _;
    let a = ranked_local::A(Some(Box::new(ranked_local::B(Some(Box::new(
        ranked_local::A(None),
    ))))));
    // inner A at depth 1: 1; B: 1 + 10 = 11; outer A: 11 + 0
    assert_eq!(a.walk(0), 11);
}

#[test]
fn structural_local_trait_named_like_the_original() {
    use crate::d::Walk as _;
    let a = structural_local::A(Some(Box::new(structural_local::B(Some(Box::new(
        structural_local::A(None),
    ))))));
    assert_eq!(a.walk(0), 11);
}

// `X` is used as `Y`, and `Y` as `Z`: the alias of one is the original name of the other.
macro_rules! chained_renames {
    ($attr:meta, $name:ident) => {
        #[$attr]
        mod $name {
            #[decycle]
            use crate::d::X as Y;
            #[decycle]
            use crate::d::Y as Z;

            pub struct A(pub Option<Box<B>>);
            pub struct B(pub Option<Box<A>>);

            // `Y` is `d::X` here and `Z` is `d::Y`.
            impl Y for A
            where
                B: Z,
            {
                fn x(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |b| b.y(depth + 1))
                }
            }

            impl Z for B
            where
                A: Y,
            {
                fn y(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |a| a.x(depth + 1) + 10)
                }
            }
        }
    };
}

chained_renames!(decycle::decycle, ranked_chain);
chained_renames!(decycle::decycle(structural), structural_chain);

#[test]
fn ranked_chained_renames() {
    use crate::d::X as _;
    let a = ranked_chain::A(Some(Box::new(ranked_chain::B(Some(Box::new(
        ranked_chain::A(None),
    ))))));
    // inner A at depth 2: 2; B: 2 + 10 = 12; outer A: 12
    assert_eq!(a.x(0), 12);
}

#[test]
fn structural_chained_renames() {
    use crate::d::X as _;
    let a = structural_chain::A(Some(Box::new(structural_chain::B(Some(Box::new(
        structural_chain::A(None),
    ))))));
    assert_eq!(a.x(0), 12);
}
