//! A `#[decycle]` trait whose supertrait is another `#[decycle]` trait of the module
//! (`trait Eval: Base`), spelled bare or `self::`.
//!
//! The ranked engine ranks such a supertrait with the trait (`EvalRanked<Rank>: BaseRanked<Rank>`),
//! so a subtrait impl's body can call the supertrait's methods on `Self` and on cycle members, at any
//! depth. Every scenario that both engines support is instantiated under both, with and without
//! `module`.

#![allow(dead_code)]

macro_rules! body_calls {
    ($m:ident [$($args:tt)*] $base:path) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Base {
                fn base(&self) -> u32;
            }

            #[decycle]
            pub trait Eval: $base {
                fn eval(&self, n: u32) -> u32;
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

            // The supertrait's method, on `Self` and on a cycle member.
            impl Eval for Expr
            where
                Term: Eval,
            {
                fn eval(&self, n: u32) -> u32 {
                    match &self.0 {
                        Some(t) => 1000 * (self.base() % 2) + t.base() + t.eval(n),
                        None => n,
                    }
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self, n: u32) -> u32 {
                    match &self.0 {
                        Some(e) => e.base() + e.eval(n),
                        None => n,
                    }
                }
            }

            pub fn generic<T: Eval>(t: &T) -> u32 {
                t.base() + t.eval(0)
            }
        }
    };
}

body_calls!(ranked_body [] Base);
body_calls!(ranked_body_self [] self::Base);
body_calls!(ranked_body_module [module = crate::ranked_body_module] Base);
body_calls!(structural_body [structural] Base);
body_calls!(structural_body_self [structural] self::Base);
// Bounded: no re-entry registry, so only a depth within `recurse_level`.
body_calls!(ranked_body_bounded [recurse_level = 12, support_infinite_cycle = false] Base);

#[test]
fn supertrait_methods_in_subtrait_bodies() {
    macro_rules! check {
        ($m:ident) => {{
            use $m::{generic, Base, Eval, Expr, Term};
            fn outside<T: Eval>(t: &T) -> u32 {
                t.base()
            }
            let nest = |depth: usize| {
                let mut e = Expr(None);
                for _ in 0..depth {
                    e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
                }
                e
            };
            let e = nest(3);
            assert_eq!(e.eval(5), 3026, "{}", stringify!($m));
            assert_eq!((e.base(), generic(&e), outside(&e)), (7, 3028, 7));
            // Far past the ranked engine's floor.
            let d = nest(60);
            assert_eq!((d.base(), outside(&d)), (121, 121), "{}", stringify!($m));
            assert!(generic(&d) > 0);
        }};
    }
    check!(ranked_body);
    check!(ranked_body_self);
    check!(ranked_body_module);
    check!(structural_body);
    check!(structural_body_self);
}

#[test]
fn supertrait_methods_in_subtrait_bodies_bounded() {
    use ranked_body_bounded::{generic, Base, Eval, Expr, Term};
    let mut e = Expr(None);
    for _ in 0..3 {
        e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
    }
    assert_eq!((e.eval(5), e.base(), generic(&e)), (3026, 7, 3028));
}

// ---- a chain of supertraits, and several ----

macro_rules! chain {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait C {
                fn c(&self) -> u32;
            }

            #[decycle]
            pub trait B: C {
                fn b(&self) -> u32;
            }

            #[decycle]
            pub trait Other {
                fn other(&self) -> u32;
            }

            #[decycle]
            pub trait A: B + Other {
                fn a(&self) -> u32;
            }

            pub struct X(pub Option<Box<Y>>);
            pub struct Y(pub Option<Box<X>>);

            impl C for X
            where
                Y: C,
            {
                fn c(&self) -> u32 {
                    self.0.as_ref().map_or(1, |y| y.c() + 1)
                }
            }

            impl C for Y
            where
                X: C,
            {
                fn c(&self) -> u32 {
                    self.0.as_ref().map_or(1, |x| x.c() + 1)
                }
            }

            impl B for X
            where
                Y: B,
            {
                fn b(&self) -> u32 {
                    self.c() + self.0.as_ref().map_or(0, |y| y.b())
                }
            }

            impl B for Y
            where
                X: B,
            {
                fn b(&self) -> u32 {
                    self.0.as_ref().map_or(0, |x| x.b())
                }
            }

            impl Other for X
            where
                Y: Other,
            {
                fn other(&self) -> u32 {
                    self.0.as_ref().map_or(10, |y| y.other())
                }
            }

            impl Other for Y
            where
                X: Other,
            {
                fn other(&self) -> u32 {
                    self.0.as_ref().map_or(10, |x| x.other())
                }
            }

            // `C` is reached through `B`: a supertrait of a supertrait.
            impl A for X
            where
                Y: A,
            {
                fn a(&self) -> u32 {
                    self.b() + self.c() + self.other() + self.0.as_ref().map_or(0, |y| y.a() + y.c())
                }
            }

            impl A for Y
            where
                X: A,
            {
                fn a(&self) -> u32 {
                    self.0.as_ref().map_or(0, |x| x.a())
                }
            }
        }
    };
}

chain!(ranked_chain []);
chain!(ranked_chain_module[module = crate::ranked_chain_module]);
chain!(structural_chain[structural]);

#[test]
fn supertrait_chains_and_several_supertraits() {
    macro_rules! check {
        ($m:ident) => {{
            use $m::{Other, A, B, C, X, Y};
            fn outside<T: A>(t: &T) -> u32 {
                t.c() + t.other()
            }
            let mut x = X(None);
            for _ in 0..40 {
                x = X(Some(Box::new(Y(Some(Box::new(x))))));
            }
            assert_eq!(
                (x.a(), x.b(), x.c()),
                (27552, 1681, 81),
                "{}",
                stringify!($m)
            );
            assert_eq!((x.other(), outside(&x)), (10, 91), "{}", stringify!($m));
        }};
    }
    check!(ranked_chain);
    check!(ranked_chain_module);
    check!(structural_chain);
}

// ---- generic impls, and a supertrait with generic arguments ----

macro_rules! generic {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Base<X> {
                fn base(&self, x: X) -> u32;
            }

            #[decycle]
            pub trait Eval: Base<u32> {
                fn eval(&self) -> u32;
            }

            pub struct Wrap<T>(pub T);
            pub struct Expr(pub Option<Box<Wrap<Term>>>);
            pub struct Term(pub Option<Box<Expr>>);

            impl<T> Base<u32> for Wrap<T>
            where
                T: Base<u32>,
            {
                fn base(&self, x: u32) -> u32 {
                    self.0.base(x)
                }
            }

            impl Base<u32> for Expr
            where
                Wrap<Term>: Base<u32>,
            {
                fn base(&self, x: u32) -> u32 {
                    self.0.as_ref().map_or(x, |w| w.base(x) + 1)
                }
            }

            impl Base<u32> for Term
            where
                Expr: Base<u32>,
            {
                fn base(&self, x: u32) -> u32 {
                    self.0.as_ref().map_or(x, |e| e.base(x) + 1)
                }
            }

            // `T: Eval` implies the `T: Base<u32>` that `Wrap<T>: Base<u32>` needs, and the body calls
            // the supertrait's method on `Self`, an impl generic over its cyclic parameter.
            impl<T> Eval for Wrap<T>
            where
                T: Eval,
            {
                fn eval(&self) -> u32 {
                    self.0.eval() + self.base(0) * 0
                }
            }

            impl Eval for Expr
            where
                Wrap<Term>: Eval,
            {
                fn eval(&self) -> u32 {
                    self.0.as_ref().map_or(0, |w| w.eval() + 1)
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self) -> u32 {
                    self.0.as_ref().map_or(0, |e| e.eval() + 1)
                }
            }
        }
    };
}

generic!(ranked_generic []);
generic!(ranked_generic_module[module = crate::ranked_generic_module]);
generic!(structural_generic[structural]);

#[test]
fn generic_impls_and_generic_supertraits() {
    macro_rules! check {
        ($m:ident) => {{
            use $m::{Base, Eval, Expr, Term, Wrap};
            let mut e = Expr(None);
            for _ in 0..30 {
                e = Expr(Some(Box::new(Wrap(Term(Some(Box::new(e)))))));
            }
            assert_eq!((e.eval(), e.base(5)), (60, 65), "{}", stringify!($m));
            assert_eq!(Wrap(Term(None)).base(7), 7, "{}", stringify!($m));
        }};
    }
    check!(ranked_generic);
    check!(ranked_generic_module);
    check!(structural_generic);
}

// ---- ranked only: supertraits the structural engine has no terminator impl for ----

// The structural engine's terminators implement only the pairs of their own cycle, so a supertrait
// outside it (acyclic, implemented elsewhere, or reached through a cross-trait edge only) is
// unproven for them (`E0277` on the terminator). The ranked engine has no such terminator.

#[decycle::decycle]
pub mod ranked_cross {
    #[decycle]
    pub trait Base {
        fn base(&self) -> u32;
    }

    #[decycle]
    pub trait Eval: Base {
        fn eval(&self) -> u32;
    }

    pub struct Expr(pub Option<Box<Term>>);
    pub struct Term(pub Option<Box<Expr>>);

    // `Base` and `Eval` reach each other only through these cross-trait bounds.
    impl Base for Expr
    where
        Term: Eval,
    {
        fn base(&self) -> u32 {
            self.0.as_ref().map_or(1, |t| t.eval() + 1)
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
        Term: Eval,
    {
        fn eval(&self) -> u32 {
            self.0.as_ref().map_or(0, |t| t.eval() + 1)
        }
    }

    impl Eval for Term
    where
        Expr: Base,
    {
        fn eval(&self) -> u32 {
            self.0.as_ref().map_or(0, |e| e.base() + 1)
        }
    }
}

#[decycle::decycle]
pub mod ranked_acyclic_super {
    #[decycle]
    pub trait Base {
        fn base(&self) -> u32;
    }

    #[decycle]
    pub trait Eval: self::Base {
        fn eval(&self) -> u32;
    }

    pub struct Expr(pub Option<Box<Term>>);
    pub struct Term(pub Option<Box<Expr>>);

    // An acyclic supertrait, called on `Self` and on a member.
    impl Base for Expr {
        fn base(&self) -> u32 {
            2
        }
    }

    impl Base for Term {
        fn base(&self) -> u32 {
            3
        }
    }

    impl Eval for Expr
    where
        Term: Eval,
    {
        fn eval(&self) -> u32 {
            self.base() + self.0.as_ref().map_or(0, |t| t.base() + t.eval())
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
}

// The supertrait implemented outside the module: it has no ranked twin for these types, so it stays
// the real trait (the subtrait's bodies can then call it only through its path).
#[decycle::decycle]
pub mod ranked_super_outside {
    #[decycle]
    pub trait Base {
        fn base(&self) -> u32;
    }

    #[decycle]
    pub trait Eval: Base {
        fn eval(&self) -> u32;
    }

    pub struct Expr(pub Option<Box<Term>>);
    pub struct Term(pub Option<Box<Expr>>);

    impl Eval for Expr
    where
        Term: Eval,
    {
        fn eval(&self) -> u32 {
            self.0.as_ref().map_or(0, |t| t.eval() + 1)
        }
    }

    impl Eval for Term
    where
        Expr: Eval,
    {
        fn eval(&self) -> u32 {
            self.0.as_ref().map_or(0, |e| e.eval() + 1)
        }
    }
}

impl ranked_super_outside::Base for ranked_super_outside::Expr {
    fn base(&self) -> u32 {
        5
    }
}

impl ranked_super_outside::Base for ranked_super_outside::Term {
    fn base(&self) -> u32 {
        6
    }
}

#[test]
fn ranked_supertraits_outside_the_cycle() {
    {
        use ranked_cross::{Base, Eval, Expr, Term};
        let mut e = Expr(None);
        for _ in 0..30 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
        }
        assert_eq!((e.eval(), e.base()), (61, 61));
    }
    {
        use ranked_acyclic_super::{Base, Eval, Expr, Term};
        let mut e = Expr(None);
        for _ in 0..30 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
        }
        assert_eq!((e.eval(), e.base()), (152, 2));
    }
    {
        use ranked_super_outside::{Base, Eval, Expr, Term};
        let mut e = Expr(None);
        for _ in 0..30 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
        }
        assert_eq!((e.eval(), e.base()), (60, 5));
    }
}
