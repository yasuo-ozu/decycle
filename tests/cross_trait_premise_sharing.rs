//! The impls of a cycle share their premises across traits, in both engines.
//!
//! A cycle may alternate between traits (`Expr: Eval` needs `Term: Size`, which needs `Expr: Eval`).
//! Each impl's generated code reaches the next one through the public trait, and that impl demands
//! its own premises (`T: Clone` on `Term`'s `Size` impl), which the impl before it need not state:
//! plain Rust proves them through the cyclic bound, which the engines strip or rank. Both engines
//! shared premises among the impls of one trait only, so `Expr` (the cycle's only `Eval` impl)
//! never received `Term`'s `T: Clone`: with `structural` its generated impl failed with `E0277`,
//! and the ranked engine skipped the re-entry registrations that need the premise, so a recursion
//! past its floor panicked (here: `three_traits`, `unshared`). Premises now travel along the
//! cycle's bounds, whatever their trait, with the generic parameters mapped through each bound
//! (`Term: Size<T>` says `Size`'s parameter is `T` there), not matched by name.
//!
//! Every scenario is expanded by both engines and checked the same way, past the ranked engine's
//! floor (40 levels).

#![allow(dead_code)]
// `impl<'u, T> .. where T: 'u`: a lifetime only a bound names, on purpose (see `unshared`).
#![allow(clippy::extra_unused_lifetimes)]

use core::fmt::Debug;

/// `Clone + Debug`, not `Default`.
#[derive(Clone, Debug)]
pub struct OnlyClone(pub u8);

/// `Default` only.
#[derive(Default)]
pub struct OnlyDefault;

/// Neither.
pub struct Bare;

/// Emit the items twice (ranked + structural) and run `$body` against each.
macro_rules! both {
    ($name:ident { $($items:tt)* } run $body:block) => {
        mod $name {
            #[decycle::decycle]
            pub mod ranked {
                #[allow(unused_imports)]
                use super::super::*;
                $($items)*
            }
            #[decycle::decycle(structural)]
            pub mod structural {
                #[allow(unused_imports)]
                use super::super::*;
                $($items)*
            }
        }
        #[test]
        fn $name() {
            {
                use $name::ranked::*;
                $body
            }
            {
                use $name::structural::*;
                $body
            }
        }
    };
}

// `Expr` is the cycle's only `Eval` impl and `Term` its only `Size` impl, so sharing per trait
// never brought `T: Clone` to `Expr`.
both! {
    probe {
        #[decycle] pub trait Eval<T> { fn eval(&self, t: &T) -> usize; }
        #[decycle] pub trait Size<T> { fn size(&self, t: &T) -> usize; }
        pub struct Expr(pub Option<Box<Term>>);
        pub struct Term(pub Option<Box<Expr>>);
        impl<T> Eval<T> for Expr where Term: Size<T> {
            fn eval(&self, t: &T) -> usize { self.0.as_ref().map_or(0, |x| x.size(t)) + 1 }
        }
        impl<T> Size<T> for Term where Expr: Eval<T>, T: Clone {
            fn size(&self, t: &T) -> usize {
                let _c = t.clone();
                self.0.as_ref().map_or(0, |x| x.eval(t)) + 1
            }
        }
    }
    run {
        let mut e = Expr(None);
        for _ in 0..20 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
        }
        assert_eq!(e.eval(&0u8), 41);
        assert_eq!(e.eval(&OnlyClone(0)), 41);
        // Deep enough to pass the ranked engine's floor many times over.
        let mut e = Expr(None);
        for _ in 0..40 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
        }
        assert_eq!(e.eval(&OnlyClone(0)), 81);
        assert_eq!(Term(Some(Box::new(e))).size(&OnlyClone(0)), 82);
    }
}

// The same with the parameter named differently on each side: the premise `U: Clone` reaches
// `Expr` as `T: Clone`, through the bound `Term: Size<T>`.
both! {
    renamed {
        #[decycle] pub trait Eval<T> { fn eval(&self, t: &T) -> usize; }
        #[decycle] pub trait Size<T> { fn size(&self, t: &T) -> usize; }
        pub struct Expr(pub Option<Box<Term>>);
        pub struct Term(pub Option<Box<Expr>>);
        impl<T> Eval<T> for Expr where Term: Size<T> {
            fn eval(&self, t: &T) -> usize { self.0.as_ref().map_or(0, |x| x.size(t)) + 1 }
        }
        impl<U> Size<U> for Term where Expr: Eval<U>, U: Clone {
            fn size(&self, t: &U) -> usize {
                let _c = t.clone();
                self.0.as_ref().map_or(0, |x| x.eval(t)) + 1
            }
        }
    }
    run {
        let mut e = Expr(None);
        for _ in 0..40 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
        }
        assert_eq!(e.eval(&OnlyClone(0)), 81);
    }
}

// Generic cycle types whose parameters are named differently too, and premises naming the self
// type: `Term<R>`'s `R: Copy` and `Self: Debug` reach `Expr<S>` as `S: Copy` and `Term<S>: Debug`.
both! {
    generic_types {
        #[decycle] pub trait Eval<T> { fn eval(&self, t: &T) -> usize; }
        #[decycle] pub trait Size<T> { fn size(&self, t: &T) -> usize; }
        #[derive(Debug)]
        pub struct Expr<S>(pub Option<Box<Term<S>>>, pub S);
        #[derive(Debug)]
        pub struct Term<R>(pub Option<Box<Expr<R>>>, pub R);
        impl<S, T> Eval<T> for Expr<S> where Term<S>: Size<T> {
            fn eval(&self, t: &T) -> usize { self.0.as_ref().map_or(0, |x| x.size(t)) + 1 }
        }
        impl<R, U> Size<U> for Term<R>
        where
            Expr<R>: Eval<U>,
            U: Clone,
            R: Copy,
            Self: Debug,
        {
            fn size(&self, t: &U) -> usize {
                let _c = t.clone();
                let _r: R = self.1;
                let _shown = format!("{:?}", self);
                self.0.as_ref().map_or(0, |x| x.eval(t)) + 1
            }
        }
    }
    run {
        let mut e = Expr(None, 7i32);
        for _ in 0..40 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e)), 7i32))), 7i32);
        }
        assert_eq!(e.eval(&OnlyClone(0)), 81);
    }
}

// Three traits in one cycle, each with its own premise: every impl needs both of the others'.
both! {
    three_traits {
        #[decycle] pub trait Eval<T> { fn eval(&self, t: &T) -> usize; }
        #[decycle] pub trait Size<T> { fn size(&self, t: &T) -> usize; }
        #[decycle] pub trait Show<T> { fn show(&self, t: &T) -> usize; }
        pub struct A(pub Option<Box<B>>);
        pub struct B(pub Option<Box<C>>);
        pub struct C(pub Option<Box<A>>);
        impl<T> Eval<T> for A where B: Size<T>, T: PartialEq {
            fn eval(&self, t: &T) -> usize {
                let _same = t.eq(t);
                self.0.as_ref().map_or(0, |x| x.size(t)) + 1
            }
        }
        impl<U> Size<U> for B where C: Show<U>, U: Default {
            fn size(&self, t: &U) -> usize {
                let _fresh = U::default();
                self.0.as_ref().map_or(0, |x| x.show(t)) + 1
            }
        }
        impl<V> Show<V> for C where A: Eval<V>, V: Clone {
            fn show(&self, t: &V) -> usize {
                let _c = t.clone();
                self.0.as_ref().map_or(0, |x| x.eval(t)) + 1
            }
        }
    }
    run {
        let mut a = A(None);
        for _ in 0..40 {
            a = A(Some(Box::new(B(Some(Box::new(C(Some(Box::new(a)))))))));
        }
        assert_eq!(a.eval(&0u8), 121);
        let b = B(Some(Box::new(C(Some(Box::new(a))))));
        assert_eq!(b.size(&0u8), 123);
    }
}

// A cross-trait cycle and a same-trait cycle in one module, plus an impl outside both: each cycle
// shares its own premises only, and the outsider gains nothing.
both! {
    mixed {
        #[decycle] pub trait Eval<T> { fn eval(&self, t: &T) -> usize; }
        #[decycle] pub trait Size<T> { fn size(&self, t: &T) -> usize; }
        pub struct Expr(pub Option<Box<Term>>);
        pub struct Term(pub Option<Box<Expr>>);
        pub struct Left(pub Option<Box<Right>>);
        pub struct Right(pub Option<Box<Left>>);
        pub struct Solo;
        impl<T> Eval<T> for Expr where Term: Size<T> {
            fn eval(&self, t: &T) -> usize { self.0.as_ref().map_or(0, |x| x.size(t)) + 1 }
        }
        impl<T> Size<T> for Term where Expr: Eval<T>, T: Clone {
            fn size(&self, t: &T) -> usize {
                let _c = t.clone();
                self.0.as_ref().map_or(0, |x| x.eval(t)) + 1
            }
        }
        impl<T> Eval<T> for Left where Right: Eval<T> {
            fn eval(&self, t: &T) -> usize { self.0.as_ref().map_or(0, |x| x.eval(t)) + 1 }
        }
        impl<T> Eval<T> for Right where Left: Eval<T>, T: Default {
            fn eval(&self, t: &T) -> usize {
                let _fresh = T::default();
                self.0.as_ref().map_or(0, |x| x.eval(t)) + 1
            }
        }
        impl<T> Eval<T> for Solo {
            fn eval(&self, _t: &T) -> usize { 7 }
        }
        impl<T> Size<T> for Solo {
            fn size(&self, _t: &T) -> usize { 8 }
        }
    }
    run {
        let mut e = Expr(None);
        for _ in 0..40 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
        }
        // `OnlyClone` is not `Default`: the `Left` cycle's premise did not reach `Expr`.
        assert_eq!(e.eval(&OnlyClone(0)), 81);
        let mut l = Left(None);
        for _ in 0..40 {
            l = Left(Some(Box::new(Right(Some(Box::new(l))))));
        }
        // `OnlyDefault` is not `Clone`: the `Expr` cycle's premise did not reach `Left`.
        assert_eq!(l.eval(&OnlyDefault), 81);
        assert_eq!(Eval::eval(&Solo, &Bare), 7);
        assert_eq!(Size::size(&Solo, &Bare), 8);
    }
}

// Premises with lifetimes, mapped by position as well: `Term<'t>`'s `'t: 'b`, `U: 'b` and
// `for<'x> &'x U: Debug` reach `Expr<'s>` (whose impl calls the trait's lifetime `'x`) as `'s: 'x`,
// `T: 'x` and a `for<..>` bound whose binder is renamed apart from the impl's own `'x`. (The ranked
// engine, whose registration check compares premises by spelling, takes the first two only.)
both! {
    lifetimes {
        #[decycle] pub trait Eval<'a, T> { fn eval(&self, t: &'a T) -> usize; }
        #[decycle] pub trait Size<'b, U> { fn size(&self, t: &'b U) -> usize; }
        pub struct Expr<'s>(pub Option<Box<Term<'s>>>, pub &'s str);
        pub struct Term<'s>(pub Option<Box<Expr<'s>>>, pub &'s str);
        impl<'s, 'x, T> Eval<'x, T> for Expr<'s> where Term<'s>: Size<'x, T> {
            fn eval(&self, t: &'x T) -> usize {
                self.0.as_ref().map_or(0, |x| x.size(t)) + self.1.len()
            }
        }
        impl<'t, 'b, U> Size<'b, U> for Term<'t>
        where
            Expr<'t>: Eval<'b, U>,
            U: 'b + Clone,
            't: 'b,
            for<'x> &'x U: Debug,
        {
            fn size(&self, t: &'b U) -> usize {
                let kept: &'b str = self.1;
                let copy = t.clone();
                let _shown = format!("{:?}", &copy);
                self.0.as_ref().map_or(0, |x| x.eval(t)) + kept.len()
            }
        }
    }
    run {
        let text = String::from("ab");
        let mut e = Expr(None, &text);
        for _ in 0..40 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e)), &text))), &text);
        }
        let v = OnlyClone(0);
        assert_eq!(e.eval(&v), 162);
    }
}

// Premises that must not be shared as written.
//
// - `Term`'s `T: 'u` names a lifetime that only `Term`'s impl has (no bound of the cycle says what
//   it is on `Expr`'s side): it is not copied, while its co-bound `T: Clone` is.
// - `Expr` reaches `Term` at `u8` only (`Term: Size<u8>`), so `Term`'s `T: Clone` is `u8: Clone`
//   on `Expr`'s side, not `T: Clone`: `Expr` still takes a `T` that is `Default` but not `Clone`.
both! {
    unshared {
        #[decycle] pub trait Eval<T> { fn eval(&self, t: &T) -> usize; }
        #[decycle] pub trait Size<T> { fn size(&self, t: &T) -> usize; }
        pub struct Expr(pub Option<Box<Term>>);
        pub struct Term(pub Option<Box<Expr>>);
        impl<T> Eval<T> for Expr where Term: Size<u8>, T: Default {
            fn eval(&self, _t: &T) -> usize {
                let _fresh = T::default();
                self.0.as_ref().map_or(0, |x| x.size(&0u8)) + 1
            }
        }
        impl<'u, T> Size<T> for Term where Expr: Eval<T>, T: Clone + 'u {
            fn size(&self, t: &T) -> usize {
                let _c = t.clone();
                self.0.as_ref().map_or(0, |x| x.eval(t)) + 1
            }
        }
        pub struct Lone(pub Option<Box<Lone>>);
        impl<'u, T> Eval<T> for Lone where Lone: Eval<T>, T: 'u {
            fn eval(&self, t: &T) -> usize { self.0.as_ref().map_or(0, |x| x.eval(t)) + 1 }
        }
    }
    run {
        let mut e = Expr(None);
        for _ in 0..40 {
            e = Expr(Some(Box::new(Term(Some(Box::new(e))))));
        }
        assert_eq!(e.eval(&OnlyDefault), 81);
        assert_eq!(Term(Some(Box::new(e))).size(&0u8), 82);
        assert_eq!(Lone(Some(Box::new(Lone(None)))).eval(&Bare), 2);
    }
}
