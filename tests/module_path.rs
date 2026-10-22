//! `#[decycle(module = path)]`: the module's own absolute path.
//!
//! The cycle is solved on the module as written, where only a bare (or `self::`) name is the
//! module's: a bound spelled by a path (`crate::m::Term: Eval`) is the opt-out, as without `module`
//! (see `ui/module_absolute_bound*.rs`). Once it is solved, the items re-emitted as written have each
//! bare or `self::` name of a type or trait declared in the module spelled `<module>::X`, so an
//! `#[announce]` on one of them (which requires absolute paths) accepts its bare fields, and keep the
//! absolute paths written in them. A path that is not the module's is an error at the argument
//! (`ui/module_wrong_path.rs`, `ui/module_missing_path.rs`).
//!
//! Every scenario is instantiated under both engines; the module paths are spelled out at each
//! instantiation, since they are what is being tested.

#![allow(dead_code)]

// Makes `::module_path_test::..` name this crate, for the leading-`::` form of `module`.
extern crate self as module_path_test;

// ---- the cycle of the issue ----

macro_rules! eval_cycle {
    ($m:ident [$($engine:ident)?] Expr: [$($expr_header:tt)*] Term: [$($term_header:tt)*]) => {
        #[decycle::decycle($($engine,)? module = crate::$m)]
        pub mod $m {
            #[decycle]
            pub trait Eval {
                fn eval(&self) -> i32;
            }

            pub enum Expr {
                Add(Box<Term>, Box<Term>),
                One,
            }

            pub enum Term {
                Paren(Box<Expr>),
                Lit(i32),
            }

            impl $($expr_header)* {
                fn eval(&self) -> i32 {
                    match self {
                        Expr::Add(a, b) => a.eval() + b.eval(),
                        Expr::One => 1,
                    }
                }
            }

            impl $($term_header)* {
                fn eval(&self) -> i32 {
                    match self {
                        Term::Paren(e) => e.eval(),
                        Term::Lit(n) => *n,
                    }
                }
            }
        }
    };
}

eval_cycle!(ranked_bare []
    Expr: [Eval for Expr where Term: Eval]
    Term: [Eval for Term where Expr: Eval]);
eval_cycle!(structural_bare [structural]
    Expr: [Eval for Expr where Term: Eval]
    Term: [Eval for Term where Expr: Eval]);

// The leading-`::` form, through `extern crate self` (a macro of its own: `eval_cycle!` writes
// `module = crate::..`).
macro_rules! leading_colon_cycle {
    ($m:ident [$($engine:ident)?]) => {
        #[decycle::decycle($($engine,)? module = ::module_path_test::$m)]
        pub mod $m {
            #[decycle]
            pub trait Eval {
                fn eval(&self) -> i32;
            }

            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub enum Expr {
                Add(::std::boxed::Box<Term>, ::std::boxed::Box<Term>),
                One,
            }

            pub enum Term {
                Paren(Box<Expr>),
                Lit(i32),
            }

            impl Eval for Expr
            where
                Term: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Expr::Add(a, b) => a.eval() + b.eval(),
                        Expr::One => 1,
                    }
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Term::Paren(e) => e.eval(),
                        Term::Lit(n) => *n,
                    }
                }
            }
        }
    };
}

leading_colon_cycle!(ranked_leading_colon []);
leading_colon_cycle!(structural_leading_colon[structural]);

macro_rules! eval_in {
    ($($m:ident),*) => {$({
        use crate::$m::{Eval, Expr, Term};
        // 2 + (3 + 1)
        let e = Expr::Add(
            Box::new(Term::Lit(2)),
            Box::new(Term::Paren(Box::new(Expr::Add(
                Box::new(Term::Lit(3)),
                Box::new(Term::Paren(Box::new(Expr::One))),
            )))),
        );
        assert_eq!(e.eval(), 6, "{}", stringify!($m));
    })*};
}

#[test]
fn bare_bounds() {
    eval_in!(ranked_bare, structural_bare);
}

#[test]
fn leading_colon_module_path() {
    eval_in!(ranked_leading_colon, structural_leading_colon);
}

// ---- `$crate`, as a `macro_rules!` that generates the module passes it ----

macro_rules! dollar_crate_module {
    ($m:ident [$($engine:ident)?]) => {
        #[decycle::decycle($($engine,)? module = $crate::$m)]
        pub mod $m {
            #[decycle]
            pub trait Eval {
                fn eval(&self) -> i32;
            }

            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub enum Expr {
                Add(::std::boxed::Box<Term>, ::std::boxed::Box<Term>),
                One,
            }

            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub enum Term {
                Paren(::std::boxed::Box<Expr>),
                Lit(i32),
            }

            impl Eval for Expr
            where
                Term: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Expr::Add(a, b) => a.eval() + b.eval(),
                        Expr::One => 1,
                    }
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Term::Paren(e) => e.eval(),
                        Term::Lit(n) => *n,
                    }
                }
            }
        }
    };
}

dollar_crate_module!(ranked_dollar_crate []);
dollar_crate_module!(structural_dollar_crate[structural]);

#[test]
fn dollar_crate_module_path() {
    eval_in!(ranked_dollar_crate, structural_dollar_crate);
}

// ---- generic module types ----

macro_rules! generic_cycle {
    ($m:ident [$($engine:ident)?]) => {
        #[decycle::decycle($($engine,)? module = crate::$m)]
        pub mod $m {
            #[decycle]
            pub trait Total {
                fn total(&self) -> i64;
            }

            pub enum List<T> {
                Cons(Box<Tree<T>>, Box<List<T>>),
                Nil,
            }

            pub enum Tree<T> {
                Leaf(T),
                Branch(Box<List<T>>),
            }

            impl<T: Copy + Into<i64>> Total for List<T>
            where
                Tree<T>: Total,
                List<T>: Total,
            {
                fn total(&self) -> i64 {
                    match self {
                        List::Cons(tree, rest) => tree.total() + rest.total(),
                        List::Nil => 0,
                    }
                }
            }

            impl<T: Copy + Into<i64>> Total for Tree<T>
            where
                List<T>: Total,
            {
                fn total(&self) -> i64 {
                    match self {
                        Tree::Leaf(v) => (*v).into(),
                        Tree::Branch(list) => list.total(),
                    }
                }
            }
        }
    };
}

generic_cycle!(ranked_generic []);
generic_cycle!(structural_generic[structural]);

#[test]
fn generic_module_types() {
    macro_rules! check {
        ($m:ident) => {{
            use crate::$m::{List, Total, Tree};
            let leaf = |v: u8| Box::new(Tree::Leaf(v));
            let inner = List::Cons(leaf(4), Box::new(List::Nil));
            let list = List::Cons(
                leaf(1),
                Box::new(List::Cons(
                    Box::new(Tree::Branch(Box::new(inner))),
                    Box::new(List::Nil),
                )),
            );
            assert_eq!(list.total(), 5, "{}", stringify!($m));
        }};
    }
    check!(ranked_generic);
    check!(structural_generic);
}

// ---- a generic parameter that shadows a module type ----

macro_rules! shadowing {
    ($m:ident [$($engine:ident)?]) => {
        #[decycle::decycle($($engine,)? module = crate::$m)]
        pub mod $m {
            #[decycle]
            pub trait Eval {
                fn eval(&self) -> i32;
            }

            pub enum Expr {
                Add(Box<Term>, Box<Term>),
                One,
            }

            pub enum Term {
                Paren(Box<Expr>),
                Lit(i32),
            }

            impl Eval for Expr
            where
                Term: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Expr::Add(a, b) => a.eval() + b.eval(),
                        Expr::One => 1,
                    }
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Term::Paren(e) => e.eval(),
                        Term::Lit(n) => *n,
                    }
                }
            }

            // The first field is the PARAMETER `Term`. Qualified, it would be the enum, and the
            // parameter unused (E0392). The second field is the enum `Expr`.
            pub struct Holder<Term>(pub Term, pub Option<Box<Expr>>);

            // `Expr` is the parameter here, and `crate::..::Expr` the enum. The absolute path is
            // left as written (an opt-out premise on the enum, not an edge), so the bound stays on
            // the enum, and `Holder<u8>` needs no `u8: Eval`.
            impl<Expr: Copy> Holder<Expr>
            where
                crate::$m::Expr: Eval,
            {
                pub fn get(&self) -> Expr {
                    self.0
                }

                pub fn inner(&self) -> i32 {
                    self.1.as_ref().map_or(0, |e| e.eval())
                }
            }

            pub trait Pick {
                fn pick<Term>(&self, t: Term) -> Term;
            }

            impl Pick for Term {
                fn pick<Term>(&self, t: Term) -> Term {
                    t
                }
            }

            pub fn first<Term>(t: Term, _: &Expr) -> Term {
                t
            }

            // A body is left as written: this local `Expr` is not the module's.
            pub fn local() -> u8 {
                struct Expr(u8);
                let e: Expr = Expr(7);
                e.0
            }

            // Nor is anything in a nested module touched.
            pub mod inner {
                pub struct Term(pub u8);
                pub fn make() -> Term {
                    Term(3)
                }
            }
        }
    };
}

shadowing!(ranked_shadowing []);
shadowing!(structural_shadowing[structural]);

#[test]
fn generic_parameter_shadows_module_type() {
    macro_rules! check {
        ($m:ident) => {{
            use crate::$m::{first, inner, local, Expr, Holder, Pick, Term};
            let h = Holder(5u8, Some(Box::new(Expr::One)));
            assert_eq!(h.get(), 5u8);
            assert_eq!(h.inner(), 1);
            assert_eq!(Term::Lit(1).pick("p"), "p");
            assert_eq!(first("x", &Expr::One), "x");
            assert_eq!(local(), 7);
            assert_eq!(inner::make().0, 3);
        }};
    }
    check!(ranked_shadowing);
    check!(structural_shadowing);
}

// ---- a trait brought in by `#[decycle] use` ----

#[decycle::decycle]
pub trait Count {
    fn count(&self) -> usize;
}

macro_rules! used_trait {
    ($m:ident [$($engine:ident)?]) => {
        #[decycle::decycle($($engine,)? module = crate::$m)]
        pub mod $m {
            #[decycle]
            use crate::Count;

            pub struct A(pub Option<Box<B>>);
            pub struct B(pub Option<Box<A>>);

            impl Count for A
            where
                B: Count,
            {
                fn count(&self) -> usize {
                    self.0.as_ref().map_or(0, |b| b.count() + 1)
                }
            }

            impl Count for B
            where
                A: Count,
            {
                fn count(&self) -> usize {
                    self.0.as_ref().map_or(0, |a| a.count() + 1)
                }
            }
        }
    };
}

used_trait!(ranked_used []);
used_trait!(structural_used[structural]);

#[test]
fn trait_from_decycle_use() {
    macro_rules! check {
        ($m:ident) => {{
            use crate::$m::{A, B};
            use crate::Count as _;
            let a = A(Some(Box::new(B(Some(Box::new(A(None)))))));
            assert_eq!(a.count(), 2, "{}", stringify!($m));
        }};
    }
    check!(ranked_used);
    check!(structural_used);
}

// ---- announced items with bare fields: the reason for `module` ----

macro_rules! announced {
    ($m:ident [$($engine:ident)?]) => {
        #[decycle::decycle($($engine,)? module = crate::$m)]
        pub mod $m {
            #[decycle]
            pub trait Eval {
                fn eval(&self) -> i32;
            }

            // `#[announce]` rejects a relative path such as the bare `Term`: these compile because
            // the module spells them `crate::..::Term` before the announcement expands.
            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub enum Expr {
                Add(::std::boxed::Box<Term>, ::std::boxed::Box<Term>),
                One,
            }

            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub enum Term {
                Paren(::std::boxed::Box<Expr>),
                Lit(i32),
            }

            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub struct Node<T>(pub T, pub ::core::option::Option<::std::boxed::Box<Node<T>>>);

            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub struct Holder<Term>(pub Term, pub Expr);

            pub trait Base {
                fn id(&self) -> u8;
            }

            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub trait Describe: Base {
                fn first(&self) -> Expr;
            }

            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub type Pair = (Expr, Term);

            impl Eval for Expr
            where
                Term: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Expr::Add(a, b) => a.eval() + b.eval(),
                        Expr::One => 1,
                    }
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Term::Paren(e) => e.eval(),
                        Term::Lit(n) => *n,
                    }
                }
            }

            impl Base for Term {
                fn id(&self) -> u8 {
                    9
                }
            }

            impl Describe for Term {
                fn first(&self) -> Expr {
                    Expr::One
                }
            }
        }
    };
}

announced!(ranked_announced []);
announced!(structural_announced[structural]);

#[test]
fn announced_items_with_bare_fields() {
    macro_rules! check {
        ($m:ident) => {{
            use crate::$m::{Base, Describe, Eval, Expr, Holder, Node, Pair, Term};
            let pair: Pair = (Expr::One, Term::Paren(Box::new(Expr::One)));
            assert_eq!(pair.0.eval() + pair.1.eval(), 2, "{}", stringify!($m));
            let t = Term::Lit(4);
            assert_eq!(t.id(), 9);
            assert_eq!(t.first().eval(), 1);
            let node = Node(1u8, Some(Box::new(Node(2u8, None))));
            assert_eq!(node.1.map(|n| n.0), Some(2));
            assert_eq!(Holder("h", Expr::One).1.eval(), 1);
        }};
    }
    check!(ranked_announced);
    check!(structural_announced);
}

// ---- the structural engine passes a module without a cycle through, still qualified ----

#[decycle::decycle(structural, module = crate::structural_no_cycle)]
pub mod structural_no_cycle {
    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub struct Wrapper(
        pub Inner,
        pub ::core::option::Option<::std::boxed::Box<Wrapper>>,
    );

    pub struct Inner(pub u8);
}

#[test]
fn structural_pass_through_is_qualified() {
    use structural_no_cycle::{Inner, Wrapper};
    let w = Wrapper(Inner(3), Some(Box::new(Wrapper(Inner(4), None))));
    assert_eq!(w.0 .0 + w.1.map_or(0, |w| w.0 .0), 7);
}

// ---- an absolute path the module writes stays absolute, also for a name it imports ----

pub mod other {
    pub struct Leaf(pub i32);
}

macro_rules! imported_absolute {
    ($m:ident [$($engine:ident)?]) => {
        #[decycle::decycle($($engine,)? module = crate::$m)]
        pub mod $m {
            use crate::other::Leaf;

            #[decycle]
            pub trait Eval {
                fn eval(&self) -> i32;
            }

            // `crate::..::Leaf` names the import; announce needs it absolute, as written.
            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub struct Holder(pub crate::$m::Leaf, pub ::std::boxed::Box<Expr>);

            pub enum Expr {
                Add(Box<Term>, Box<Term>),
                One,
            }

            pub enum Term {
                Paren(Box<Expr>),
                Lit(i32),
            }

            impl Eval for Expr
            where
                Term: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Expr::Add(a, b) => a.eval() + b.eval(),
                        Expr::One => 1,
                    }
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self) -> i32 {
                    match self {
                        Term::Paren(e) => e.eval(),
                        Term::Lit(n) => *n,
                    }
                }
            }

            impl Holder {
                pub fn total(&self) -> i32 {
                    let Leaf(n) = &self.0;
                    n + self.1.eval()
                }
            }
        }
    };
}

imported_absolute!(ranked_imported_absolute []);
imported_absolute!(structural_imported_absolute[structural]);

#[test]
fn absolute_paths_to_imports_stay_absolute() {
    use crate::other::Leaf;
    let h =
        ranked_imported_absolute::Holder(Leaf(2), Box::new(ranked_imported_absolute::Expr::One));
    assert_eq!(h.total(), 3);
    let h = structural_imported_absolute::Holder(
        Leaf(2),
        Box::new(structural_imported_absolute::Expr::One),
    );
    assert_eq!(h.total(), 3);
}
