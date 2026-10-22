//! The spellings that pin the rank of a call the floor used to take by elimination.
//!
//! `self.base()` in an impl of another trait with no `Self: Base` bound, and a method call
//! through an impl of the module whose step impl cannot apply (a pair impl with a part implemented
//! outside the module), leave the rank to inference with only the floors left. The floor was then
//! selected by elimination (on the next-generation trait solver for the first case, on every solver
//! for the second) and the call panicked at run time. Each floor now has a decoy twin, so such a
//! call is `E0283` (`tests/ui/ranked_floor_by_elimination*.rs`). The spellings below pin the rank
//! and must keep compiling and running past the floor, on every solver: a `Self: Base` bound, a
//! supertrait (`trait Eval: Base`), and the call by path, through the public trait. (For the pair,
//! the public trait's impl needs every part implemented in the module, or the pair impl outside it:
//! with the pair impl in the module and a part implemented outside, the call by path is `E0277`.)

#![allow(dead_code)]

/// A `Base` cycle through `Expr` and `Term`, and an `Eval` cycle whose `Expr` impl calls
/// `self.base()` (the `$call` spelling) under the extra `$bounds`. `$super` is `Eval`'s supertrait
/// list.
macro_rules! g0 {
    ($m:ident [$($args:tt)*] [$($super:tt)*] [$($bounds:tt)*] |$s:ident| $call:expr) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Base {
                fn base(&self) -> u32;
            }

            #[decycle]
            pub trait Eval $($super)* {
                fn eval(&self) -> u32;
            }

            pub struct Expr(pub Option<Box<Term>>);
            pub struct Term(pub Option<Box<Expr>>);

            impl Base for Expr
            where
                Term: Base,
            {
                fn base(&self) -> u32 {
                    self.0.as_ref().map_or(1, |t| t.base() + 1)
                }
            }

            impl Base for Term
            where
                Expr: Base,
            {
                fn base(&self) -> u32 {
                    self.0.as_ref().map_or(1, |e| e.base() + 1)
                }
            }

            impl Eval for Expr
            where
                Term: Eval + Base,
                $($bounds)*
            {
                fn eval(&$s) -> u32 {
                    $call + $s.0.as_ref().map_or(0, |t| t.base() + t.eval())
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self) -> u32 {
                    self.0.as_ref().map_or(0, |e| e.eval())
                }
            }

            /// `Expr` nested `depth` times through `Term`.
            pub fn nest(depth: u32) -> Expr {
                let mut e = Expr(None);
                for _ in 0..depth {
                    e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
                }
                e
            }
        }
    };
}

g0!(self_bound [recurse_level = 2] [] [Self: Base,] |self| self.base());
g0!(supertrait [recurse_level = 2] [: Base] [] |self| self.base());
g0!(by_path [recurse_level = 2] [] [] |self| crate::by_path::Base::base(self));
g0!(self_bound_bounded [support_infinite_cycle = false] [] [Self: Base,] |self| self.base());
g0!(supertrait_bounded [support_infinite_cycle = false] [: Base] [] |self| self.base());
g0!(by_path_bounded [support_infinite_cycle = false] [] []
    |self| crate::by_path_bounded::Base::base(self));

/// `(eval, base)` of `Expr` nested `depth` times, computed directly: `base` grows by 2 per level,
/// and each level adds its own `base` plus its `Term`'s to `eval`.
fn expected(depth: u32) -> (u32, u32) {
    let (mut eval, mut base) = (1, 1);
    for _ in 0..depth {
        base += 2;
        eval += base + (base - 1);
    }
    (eval, base)
}

/// Unbounded, with `recurse_level = 2`: depth 30 crosses the floor many times, in both cycles.
#[test]
fn unbounded_past_the_floor() {
    for depth in [0, 1, 2, 3, 30] {
        let want = expected(depth);
        {
            use self_bound::*;
            let e = nest(depth);
            assert_eq!((e.eval(), e.base()), want, "self_bound, depth {depth}");
        }
        {
            use supertrait::*;
            let e = nest(depth);
            assert_eq!((e.eval(), e.base()), want, "supertrait, depth {depth}");
        }
        {
            use by_path::*;
            let e = nest(depth);
            assert_eq!((e.eval(), e.base()), want, "by_path, depth {depth}");
        }
    }
}

/// Bounded, at the default `recurse_level`, within its depth.
#[test]
fn bounded_within_the_limit() {
    for depth in [0, 1, 2] {
        let want = expected(depth);
        {
            use self_bound_bounded::*;
            let e = nest(depth);
            assert_eq!(
                (e.eval(), e.base()),
                want,
                "self_bound_bounded, depth {depth}"
            );
        }
        {
            use supertrait_bounded::*;
            let e = nest(depth);
            assert_eq!(
                (e.eval(), e.base()),
                want,
                "supertrait_bounded, depth {depth}"
            );
        }
        {
            use by_path_bounded::*;
            let e = nest(depth);
            assert_eq!((e.eval(), e.base()), want, "by_path_bounded, depth {depth}");
        }
    }
}

pub mod other {
    pub struct Leaf;
}

/// A pair impl in the module, called by path (`<(B, Leaf) as crate::tuple_in::Loop>::step`) and
/// so through the public trait, whose pair impl needs every part's ranked impl: the foreign part's
/// impl is in the module too, bare through a `use`.
#[decycle::decycle]
pub mod tuple_in {
    use crate::other::Leaf;

    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n)
        }
    }

    impl Loop for A
    where
        (B, Leaf): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                <(B, Leaf) as crate::tuple_in::Loop>::step(&(B, Leaf), n - 1)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }

    impl Loop for Leaf {
        fn step(&self, _n: u32) -> u32 {
            100
        }
    }
}

/// The foreign part's impl stays outside the module, and so does the pair impl, called by path.
#[decycle::decycle]
pub mod tuple_out {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        (B, crate::other::Leaf): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                crate::tuple_out::Loop::step(&(B, crate::other::Leaf), n - 1)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

impl<X: tuple_out::Loop, Y: tuple_out::Loop> tuple_out::Loop for (X, Y) {
    fn step(&self, n: u32) -> u32 {
        self.0.step(n) + self.1.step(n)
    }
}

impl tuple_out::Loop for other::Leaf {
    fn step(&self, _n: u32) -> u32 {
        100
    }
}

/// `A.step(n)` is `B.step(n - 1) + 100` and `B.step(m)` is `A.step(m - 1) + 1`, so two levels add
/// 101: 302 at 5, 2020 at 40 (past the floor of the default `recurse_level`, 10).
#[test]
fn pair_by_path_past_the_floor() {
    for (n, want) in [(0, 0), (1, 100), (2, 101), (5, 302), (40, 2020)] {
        assert_eq!(
            tuple_in::Loop::step(&tuple_in::A, n),
            want,
            "tuple_in, n = {n}"
        );
        assert_eq!(
            tuple_out::Loop::step(&tuple_out::A, n),
            want,
            "tuple_out, n = {n}"
        );
    }
}
