//! The value-generic growth check reads an impl's self type as the engines do: a qualified-self
//! type (`impl Eval for <Q as P>::Out`) is no cycle member, even where its last segment spells one
//! (`Out`), so the check leaves its methods alone. Its method below reborrows its by-value parameter
//! into the cycle's method, which the check rejects in a member's impl; it used to be rejected here
//! too, by the member `Out` it names nothing of. The instantiations stop at `&mut S`: the members
//! take theirs by value. (The ranked engine takes the impl over with the cycle's, so it states the
//! `B: Eval` it calls through.)

pub trait Src {
    fn tick(&mut self) -> u32;
}
impl Src for u32 {
    fn tick(&mut self) -> u32 {
        *self
    }
}
impl<T: Src + ?Sized> Src for &mut T {
    fn tick(&mut self) -> u32 {
        (**self).tick()
    }
}

pub trait P {
    type Out;
}
pub struct Q;
pub mod other {
    pub struct Out;
}
impl P for Q {
    type Out = other::Out;
}

macro_rules! module {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            use crate::Src;

            #[decycle]
            pub trait Eval {
                fn eval<S: Src>(src: S, depth: u32) -> u32;
            }

            pub struct Out;
            pub struct B;

            impl Eval for Out
            where
                B: Eval,
            {
                fn eval<S: Src>(src: S, depth: u32) -> u32 {
                    if depth == 0 {
                        0
                    } else {
                        <B as Eval>::eval(src, depth - 1) + 1
                    }
                }
            }

            impl Eval for B
            where
                Out: Eval,
            {
                fn eval<S: Src>(mut src: S, depth: u32) -> u32 {
                    if depth == 0 {
                        src.tick()
                    } else {
                        <Out as Eval>::eval(src, depth - 1) + 10
                    }
                }
            }

            impl Eval for <crate::Q as crate::P>::Out
            where
                B: Eval,
            {
                fn eval<S: Src>(mut src: S, depth: u32) -> u32 {
                    if depth == 0 {
                        return src.tick();
                    }
                    <B as Eval>::eval(&mut src, depth - 1)
                }
            }
        }
    };
}

module!(ranked []);
module!(structural[structural]);

#[test]
fn a_qualified_self_impl_is_not_a_member() {
    // other::Out → B (11 levels of 10 and 1, alternating) → … → B at depth 0 ticks 7.
    fn reference(depth: u32) -> u32 {
        if depth == 0 {
            7
        } else {
            b(depth - 1)
        }
    }
    fn b(depth: u32) -> u32 {
        if depth == 0 {
            7
        } else {
            out(depth - 1) + 10
        }
    }
    fn out(depth: u32) -> u32 {
        if depth == 0 {
            0
        } else {
            b(depth - 1) + 1
        }
    }
    for depth in [0, 1, 2, 3, 10, 41] {
        assert_eq!(
            <other::Out as ranked::Eval>::eval(7u32, depth),
            reference(depth),
            "ranked({depth})"
        );
        assert_eq!(
            <other::Out as structural::Eval>::eval(7u32, depth),
            reference(depth),
            "structural({depth})"
        );
    }
}
