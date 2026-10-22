//! The impls of one cycle share their leaf premises, in both engines.
//!
//! A member's generated impl reaches a sibling through the public trait, which the sibling's own
//! impl discharges — with the sibling's leaf premises. A member need not state them (here `Stmt`
//! reaches `Group`, whose impl alone needs `A: Clone` and `for<'x> &'x A: Debug`): plain Rust proves
//! them through the cyclic bound, which the engines strip or rank. So each engine gives the members
//! of a cycle the union of each other's premises; an impl outside the cycle, and a second cycle of
//! the same trait, gain nothing.
//!
//! The module is shaped as a generator such as syan writes it: the trait comes in through
//! `#[decycle] use`, impl headers name it bare, a bound is bare when its type names a type of the
//! module and fully qualified otherwise (`crate::lexer::Integer: crate::parse::Parse<A>`), and the
//! cycle runs through `Box`/`Vec` and a private substruct.

#![allow(dead_code)]

pub mod lexer {
    #[derive(Clone, Debug)]
    pub struct Integer(pub i64);
}

pub mod parse {
    /// A parser over input `A`.
    #[decycle::decycle]
    pub trait Parse<A> {
        fn parse(&self, input: &A) -> i64;
    }

    impl<A, T: crate::parse::Parse<A>> crate::parse::Parse<A> for ::std::boxed::Box<T> {
        fn parse(&self, input: &A) -> i64 {
            (**self).parse(input)
        }
    }

    impl<A, T: crate::parse::Parse<A>> crate::parse::Parse<A> for ::std::vec::Vec<T> {
        fn parse(&self, input: &A) -> i64 {
            self.iter().map(|t| t.parse(input)).sum()
        }
    }

    impl<A> crate::parse::Parse<A> for crate::lexer::Integer {
        fn parse(&self, _input: &A) -> i64 {
            self.0
        }
    }
}

macro_rules! grammar {
    ($m:ident [$($engine:ident)?]) => {
        #[decycle::decycle($($engine)?)]
        pub(crate) mod $m {
            #[decycle]
            use crate::parse::Parse;

            pub(crate) enum Expr {
                Lit(crate::lexer::Integer),
                Neg(Box<Expr>),
                Block(Vec<Stmt>),
            }

            pub(crate) enum Stmt {
                Expr(Expr),
                Group(Group),
            }

            /// A private substruct on the cycle, as a `#[group]` attribute generates one.
            pub(crate) struct Group {
                pub(crate) items: Vec<Expr>,
                pub(crate) scale: i64,
            }

            // A foreign leaf type: a premise, fully qualified.
            impl<A> Parse<A> for Expr
            where
                Box<Expr>: Parse<A>,
                Vec<Stmt>: Parse<A>,
                crate::lexer::Integer: crate::parse::Parse<A>,
            {
                fn parse(&self, input: &A) -> i64 {
                    match self {
                        Expr::Lit(i) => crate::parse::Parse::parse(i, input),
                        Expr::Neg(e) => -e.parse(input),
                        Expr::Block(stmts) => stmts.iter().map(|s| s.parse(input)).sum(),
                    }
                }
            }

            // States none of `Group`'s leaf premises, yet reaches `Group` through the trait.
            impl<A> Parse<A> for Stmt
            where
                Expr: Parse<A>,
                Group: Parse<A>,
            {
                fn parse(&self, input: &A) -> i64 {
                    match self {
                        Stmt::Expr(e) => e.parse(input),
                        Stmt::Group(g) => g.parse(input),
                    }
                }
            }

            // Leaf premises of its own: a plain bound and one with a binder lifetime.
            impl<A> Parse<A> for Group
            where
                Vec<Expr>: Parse<A>,
                A: ::core::clone::Clone,
                for<'x> &'x A: ::core::fmt::Debug,
            {
                fn parse(&self, input: &A) -> i64 {
                    let copy = input.clone();
                    let shown = ::std::format!("{:?}", &copy).len() as i64;
                    self.scale * self.items.iter().map(|e| e.parse(&copy)).sum::<i64>() + 0 * shown
                }
            }

            // Outside every cycle: gains nothing.
            pub(crate) struct Solo;

            impl<A> Parse<A> for Solo {
                fn parse(&self, _input: &A) -> i64 {
                    7
                }
            }

            // A second, disjoint cycle of the same trait, with a premise of its own.
            pub(crate) struct Left(pub(crate) Option<Box<Right>>);
            pub(crate) struct Right(pub(crate) Option<Box<Left>>);

            impl<A> Parse<A> for Left
            where
                Right: Parse<A>,
            {
                fn parse(&self, input: &A) -> i64 {
                    self.0.as_ref().map_or(0, |r| r.parse(input) + 1)
                }
            }

            impl<A> Parse<A> for Right
            where
                Left: Parse<A>,
                A: ::core::default::Default,
            {
                fn parse(&self, input: &A) -> i64 {
                    let _fresh = A::default();
                    self.0.as_ref().map_or(0, |l| l.parse(input) + 1)
                }
            }
        }
    };
}

grammar!(ranked []);
grammar!(structural[structural]);

/// `Clone + Debug`, not `Default`: what the `Expr` cycle needs, and not the `Left` cycle's premise.
#[derive(Clone, Debug)]
struct Tok(u8);

/// `Default` only: what the `Left` cycle needs, and none of the `Expr` cycle's premises.
#[derive(Default)]
struct OnlyDefault;

/// Neither.
struct Bare;

macro_rules! check {
    ($m:ident) => {{
        use crate::lexer::Integer;
        use crate::parse::Parse;
        use $m::{Expr, Group, Left, Right, Solo, Stmt};
        // -(1 + 2 * (3 + 4)) nested, then deep enough to pass the ranked engine's floor.
        let group = Group {
            items: vec![Expr::Lit(Integer(3)), Expr::Lit(Integer(4))],
            scale: 2,
        };
        let e = Expr::Neg(Box::new(Expr::Block(vec![
            Stmt::Expr(Expr::Lit(Integer(1))),
            Stmt::Group(group),
        ])));
        assert_eq!(e.parse(&Tok(0)), -15, "{}", stringify!($m));
        let mut deep = Expr::Lit(Integer(1));
        for _ in 0..40 {
            deep = Expr::Block(vec![Stmt::Group(Group {
                items: vec![deep],
                scale: 1,
            })]);
        }
        assert_eq!(deep.parse(&Tok(0)), 1, "{}", stringify!($m));
        // The impl outside the cycle requires nothing of `A`.
        assert_eq!(Solo.parse(&Bare), 7);
        // Neither cycle took the other's premises.
        let lr = Left(Some(Box::new(Right(Some(Box::new(Left(None)))))));
        assert_eq!(lr.parse(&OnlyDefault), 2, "{}", stringify!($m));
    }};
}

#[test]
fn ranked_cycle_members_share_their_premises() {
    check!(ranked);
}

#[test]
fn structural_cycle_members_share_their_premises() {
    check!(structural);
}
