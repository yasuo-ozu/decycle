//! Ranked: the generated `__Fp` fn-pointer alias must carry the **trait's** type-param bounds, not
//! only the method's.
//!
//! `emit_reentry_items` declares both sets on the alias with [`DeclBounds::Unsized`] — every bound
//! stripped — then re-states bounds in the alias's `where` clause. It used to re-state only the
//! METHOD's (`m_tycon_alias`), which is fine while the trait's params are merely *named*, but not
//! when a bound **projects through** one. Without the fix:
//!
//! ```text
//! error[E0277]: the trait bound `A: HasOut` is not satisfied
//!   --> tests/ranked_alias_projects_trait_param.rs:17:1
//!    | #[decycle]
//! ```
//!
//! # What the shape has to contain, and why
//!
//! Three ingredients, each necessary — two weaker repros failed to reproduce before this one landed:
//!
//! 1. **`type Error` used in the return type.** `-> Result<i64, Self::Error>` becomes
//!    `<DclSelf as EvalRanked<_, A>>::Error` in the alias, so `A` appears in the alias's *signature
//!    types*. Without that, `tmask` drops `A` from the alias's parameter list entirely and there is
//!    no bound to be missing. Return a plain `i64` and the bug vanishes.
//! 2. **An associated function** (no `self`), so the alias is keyed on `DclSelf` rather than a
//!    receiver — this is what syan's `Parse::parse_stream` is.
//! 3. **A method-generic bound that projects through the trait param** — `L: Sink<Out<A>>`, where
//!    `Out<A> = <A as HasOut>::Out`. This is the predicate that needs `A: HasOut` to be well-formed.
//!
//! `S: Src<Atom = A>` mirrors syan's `S: ParseStream<Atom = Atom>` and keeps the shape faithful.
//!
//! # Why it cannot be worked around downstream
//!
//! Restating `where A: HasOut` on the *method* does not help: the same code path drops method-level
//! `where` clauses onto the alias too. Verified against syan, where the equivalent edit left all
//! five errors in place. So decycle is the only place this is fixable.
//!
//! This is the ranked twin of `structural_body_trait_where.rs` — the same defect in the other
//! engine, found separately. Real-world instance: syan's `Parse::parse_stream` gaining
//! `L: ErrorLogger<ParseError<SpanOf<Atom>>>`, which broke every `#[recurse]` module.
#![allow(dead_code)]
use decycle::decycle;

pub trait HasOut {
    type Out;
}
pub struct Tag;
impl HasOut for Tag {
    type Out = i64;
}
pub type Out<A> = <A as HasOut>::Out;

pub trait Src {
    type Atom;
}
pub struct S0;
impl Src for S0 {
    type Atom = Tag;
}

pub trait Sink<T> {
    fn put(&mut self, v: T);
}
impl<T> Sink<T> for () {
    fn put(&mut self, _: T) {}
}
impl<T> Sink<T> for Vec<T> {
    fn put(&mut self, v: T) {
        self.push(v)
    }
}

#[decycle]
mod m {
    use super::{HasOut, Out, Sink, Src};

    #[decycle]
    pub trait Eval<A: HasOut> {
        type Error;
        fn eval<S: Src<Atom = A>, L: Sink<Out<A>>>(
            src: &mut S,
            sink: &mut L,
        ) -> Result<i64, Self::Error>;
    }

    pub struct Lit;
    pub struct Sum;

    impl<A: HasOut<Out = i64>> Eval<A> for Lit {
        type Error = ();
        fn eval<S: Src<Atom = A>, L: Sink<Out<A>>>(
            _src: &mut S,
            sink: &mut L,
        ) -> Result<i64, Self::Error> {
            sink.put(1);
            Ok(1)
        }
    }

    impl<A: HasOut<Out = i64>> Eval<A> for Sum
    where
        Lit: Eval<A>,
    {
        type Error = ();
        fn eval<S: Src<Atom = A>, L: Sink<Out<A>>>(
            src: &mut S,
            sink: &mut L,
        ) -> Result<i64, Self::Error> {
            sink.put(4);
            Ok(<Lit as Eval<A>>::eval(src, sink).unwrap_or(0) + 4)
        }
    }
}

#[test]
fn a_method_generic_bound_may_project_through_a_trait_param() {
    use m::Eval;
    let mut src = S0;
    assert_eq!(<m::Sum as Eval<Tag>>::eval(&mut src, &mut ()), Ok(5));
    // The collecting sink proves the parameter is genuinely threaded through the cycle
    // rather than discarded — 4 from `Sum`, then 1 from `Lit` across the cycle edge.
    let mut got: Vec<i64> = Vec::new();
    assert_eq!(<m::Sum as Eval<Tag>>::eval(&mut src, &mut got), Ok(5));
    assert_eq!(got, vec![4, 1]);
}
