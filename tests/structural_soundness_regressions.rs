//! Regression tests for the structural engine (`#[decycle(structural)]`) audit fixes:
//!  * by-value `Self` returns carrying a `Box` must not be Stacked-Borrows UB (the `__decycle_cast`
//!    `ManuallyDrop` fix) — run under miri to actually check: `cargo +nightly miri test --test
//!    structural_soundness_regressions`;
//!  * `mut` / `ref` parameter patterns must compile (they used to land in a bodiless trait decl);
//!  * a mixed cyclic + non-cyclic co-bound (`Expr: Eval + Clone`) must keep the co-bound;
//!  * generated identifiers must not collide with a user parameter literally named `__decycle_arg1`.

use decycle::decycle;

// ---- #1: by-value Self / owned-receiver casts over a Box-bearing type (miri UB regression) ----
#[decycle(structural)]
mod grow_ast {
    #[decycle]
    pub trait Grow {
        /// returns a fresh, deeper `Self` BY VALUE — a Box-bearing value crossing `__decycle_cast`.
        fn grow(&self) -> Self
        where
            Self: Sized;
        fn depth(&self) -> i64;
        /// owned receiver carrying a Box, also crosses the by-value cast.
        fn into_depth(self) -> i64
        where
            Self: Sized;
    }

    pub enum Expr {
        Lit(i64),
        Node(Option<Box<Expr>>),
    }

    impl Grow for Expr
    where
        Expr: Grow,
    {
        fn grow(&self) -> Self {
            match self {
                Expr::Lit(n) => Expr::Node(Some(Box::new(Expr::Lit(*n)))),
                Expr::Node(c) => Expr::Node(Some(Box::new(match c {
                    Some(b) => b.grow(),
                    None => Expr::Lit(0),
                }))),
            }
        }
        fn depth(&self) -> i64 {
            match self {
                Expr::Lit(_) => 0,
                Expr::Node(c) => 1 + c.as_ref().map_or(0, |b| b.depth()),
            }
        }
        fn into_depth(self) -> i64 {
            self.depth()
        }
    }
}

#[test]
fn box_bearing_self_return_is_sound() {
    use grow_ast::{Expr, Grow};
    let e = Expr::Node(Some(Box::new(Expr::Lit(3))));
    assert_eq!(e.depth(), 1);
    let g = e.grow(); // by-value Box-bearing Self through the cast
    assert_eq!(g.depth(), 2);
    let g2 = g.grow().grow();
    assert_eq!(g2.depth(), 4);
    // owned receiver over a Box-bearing value
    assert_eq!(Expr::Node(Some(Box::new(Expr::Lit(0)))).into_depth(), 1);
}

// ---- #2: mut / ref parameter patterns ----
#[decycle(structural)]
mod params {
    #[decycle]
    pub trait Calc {
        // trait decl carries plain params (patterns aren't allowed in a bodiless decl); the IMPL is
        // where `mut` / `ref` bindings appear, and decycle must not copy those into its `__run` decl.
        fn calc(&self, acc: i64, add: i64) -> i64;
    }
    pub struct N(pub i64);
    impl Calc for N
    where
        N: Calc,
    {
        fn calc(&self, mut acc: i64, ref add: i64) -> i64 {
            acc += *add + self.0;
            acc
        }
    }
}

#[test]
fn mut_and_ref_params_compile_and_run() {
    use params::{Calc, N};
    assert_eq!(N(10).calc(1, 2), 13);
}

// ---- #3: mixed cyclic + non-cyclic co-bound must keep the co-bound ----
#[decycle(structural)]
mod cobound {
    #[decycle]
    pub trait Eval {
        fn eval(&self) -> i64;
    }
    #[derive(Clone)]
    pub struct W(pub i64);
    // `W: Eval` is the cyclic bound; `W: Clone` is a co-bound that must survive onto the reduced impls.
    impl Eval for W
    where
        W: Eval + Clone,
    {
        fn eval(&self) -> i64 {
            let c = self.clone(); // uses the Clone co-bound
            c.0
        }
    }
}

#[test]
fn mixed_cobound_survives() {
    use cobound::{Eval, W};
    assert_eq!(W(7).eval(), 7);
}

// ---- #4: a user parameter literally named `__decycle_arg1` must not collide with generated idents ----
#[decycle(structural)]
mod hygiene {
    #[decycle]
    pub trait Pair {
        fn pair(&self, xy: (i64, i64), __decycle_arg1: i64) -> i64;
    }
    pub struct P;
    impl Pair for P
    where
        P: Pair,
    {
        fn pair(&self, (x, y): (i64, i64), __decycle_arg1: i64) -> i64 {
            x + y + __decycle_arg1
        }
    }
}

#[test]
fn user_ident_named_like_generated_does_not_collide() {
    use hygiene::{Pair, P};
    assert_eq!(P.pair((1, 2), 3), 6);
}

// ---- README "Supported" list: receiver shapes, method generics, APIT, assoc items (were untested) ----
#[decycle(structural)]
mod coverage {
    #[decycle]
    pub trait Node {
        type Tag;
        const K: i64;
        fn sum(&self) -> i64;
        fn bump(&mut self); // &mut self
        fn consume(self: Box<Self>) -> i64; // self: Box<Self> (Box-bearing owned receiver)
        fn mapped<F: Fn(i64) -> i64>(&self, f: F) -> i64; // method type generic
        fn via(&self, g: impl Fn(i64) -> i64) -> i64; // argument-position impl Trait
    }
    pub struct T {
        pub v: i64,
        pub next: Option<Box<T>>,
    }
    impl Node for T
    where
        T: Node,
    {
        type Tag = ();
        const K: i64 = 7;
        fn sum(&self) -> i64 {
            self.v + self.next.as_ref().map_or(0, |n| n.sum())
        }
        fn bump(&mut self) {
            self.v += 1;
            if let Some(n) = self.next.as_mut() {
                n.bump();
            }
        }
        fn consume(self: Box<Self>) -> i64 {
            self.sum()
        }
        fn mapped<F: Fn(i64) -> i64>(&self, f: F) -> i64 {
            f(self.sum())
        }
        fn via(&self, g: impl Fn(i64) -> i64) -> i64 {
            g(self.sum())
        }
    }
}

#[test]
fn receiver_shapes_generics_apit_and_assoc_items() {
    use coverage::{Node, T};
    let mk = || T {
        v: 1,
        next: Some(Box::new(T {
            v: 2,
            next: Some(Box::new(T { v: 3, next: None })),
        })),
    };
    assert_eq!(mk().sum(), 6);
    assert_eq!(<T as Node>::K, 7);
    let mut t = mk();
    t.bump(); // &mut self, recursively
    assert_eq!(t.sum(), 9); // each of 3 nodes bumped by 1
    assert_eq!(Box::new(mk()).consume(), 6); // self: Box<Self>
    assert_eq!(mk().mapped(|x| x * 10), 60); // method generic
    assert_eq!(mk().via(|x| x + 100), 106); // argument impl Trait
}
