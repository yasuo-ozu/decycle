//! Structural: the local `__DecycleBody` trait declaration must re-state the reduced impl's
//! where-predicates.
//!
//! `rec_method` renders the body trait's generic params [`Bare`] — every bound dropped — because a
//! param bound may itself be the cyclic one, and re-stating that would put the cycle back. But
//! `__run`'s signature is copied from the user's method, so if it **projects through a param**
//! (`-> <A as HasOut>::Out`, or a type argument `Wrap<<A as HasOut>::Out>`) the declaration was
//! ill-formed on its own:
//!
//! ```text
//! error[E0277]: the trait bound `A: HasOut` is not satisfied
//! ```
//!
//! …even though every impl and the one call site could prove it. The fix re-states the *reduced*
//! predicates (cycle-free by construction, and provable at the call site, which sits inside the
//! terminator impl that carries exactly them).
//!
//! Both cyclic traits here take their own generic param, since that is the shape that hurt: a bound
//! written in the impl's `where` clause rather than inline on the param declaration is invisible to
//! `params_decl` no matter which `DeclBounds` mode it uses.
#![allow(dead_code)]

use decycle::decycle;

pub trait HasOut {
    type Out;
}
pub struct Tag;
impl HasOut for Tag {
    type Out = i64;
}

pub struct Wrap<T>(pub T);

#[decycle(structural)]
mod ret {
    use super::{HasOut, Wrap};

    #[decycle]
    pub trait Eval<A: HasOut> {
        /// The return type projects through `A`, whose bound lives in the impl's `where` clause.
        fn eval(&self) -> <A as HasOut>::Out;
        /// …and again nested inside a type argument, which the old code also failed on.
        fn wrapped(&self) -> Wrap<<A as HasOut>::Out>;
    }

    pub struct Lit(pub i64);
    pub struct Sum(pub Box<Lit>, pub Box<Sum2>);
    pub struct Sum2(pub i64);

    impl<A> Eval<A> for Lit
    where
        A: HasOut<Out = i64>,
    {
        fn eval(&self) -> <A as HasOut>::Out {
            self.0
        }
        fn wrapped(&self) -> Wrap<<A as HasOut>::Out> {
            Wrap(self.0)
        }
    }

    impl<A> Eval<A> for Sum
    where
        A: HasOut<Out = i64>,
        Lit: Eval<A>,
        Sum2: Eval<A>,
    {
        fn eval(&self) -> <A as HasOut>::Out {
            <Lit as Eval<A>>::eval(&self.0) + <Sum2 as Eval<A>>::eval(&self.1)
        }
        fn wrapped(&self) -> Wrap<<A as HasOut>::Out> {
            Wrap(<Self as Eval<A>>::eval(self))
        }
    }

    impl<A> Eval<A> for Sum2
    where
        A: HasOut<Out = i64>,
        Sum: Eval<A>,
    {
        fn eval(&self) -> <A as HasOut>::Out {
            self.0
        }
        fn wrapped(&self) -> Wrap<<A as HasOut>::Out> {
            Wrap(self.0)
        }
    }
}

#[test]
fn a_return_type_may_project_through_a_param_bound() {
    use ret::Eval;
    let e = ret::Sum(Box::new(ret::Lit(3)), Box::new(ret::Sum2(4)));
    assert_eq!(<_ as Eval<Tag>>::eval(&e), 7);
    assert_eq!(<_ as Eval<Tag>>::wrapped(&e).0, 7);
}

#[test]
fn the_cycle_still_works_through_a_leaf() {
    use ret::Eval;
    assert_eq!(<_ as Eval<Tag>>::eval(&ret::Lit(9)), 9);
}

// An **argument** position projects too, and a where-predicate that is not about a param at all
// (`Wrap<A>: Clone`) has to survive onto the declaration as well without being mistaken for a cycle
// edge.
#[decycle(structural)]
mod arg {
    use super::{HasOut, Wrap};

    #[decycle]
    pub trait Fold<A: HasOut> {
        fn fold(&self, seed: <A as HasOut>::Out, tag: Wrap<i64>) -> i64;
    }

    pub struct Node(pub i64, pub Box<Leaf>);
    pub struct Leaf(pub i64);

    impl<A> Fold<A> for Node
    where
        A: HasOut<Out = i64>,
        Wrap<i64>: Clone,
        Leaf: Fold<A>,
    {
        fn fold(&self, seed: <A as HasOut>::Out, tag: Wrap<i64>) -> i64 {
            self.0 + seed + <Leaf as Fold<A>>::fold(&self.1, seed, tag.clone()) + tag.0
        }
    }

    impl<A> Fold<A> for Leaf
    where
        A: HasOut<Out = i64>,
        Node: Fold<A>,
    {
        fn fold(&self, seed: <A as HasOut>::Out, _tag: Wrap<i64>) -> i64 {
            self.0 + seed
        }
    }
}

impl Clone for Wrap<i64> {
    fn clone(&self) -> Self {
        Wrap(self.0)
    }
}

#[test]
fn an_argument_may_project_through_a_param_bound() {
    use arg::Fold;
    let n = arg::Node(1, Box::new(arg::Leaf(2)));
    // 1 + 10 + (2 + 10) + 100
    assert_eq!(<_ as Fold<Tag>>::fold(&n, 10, Wrap(100)), 123);
}
