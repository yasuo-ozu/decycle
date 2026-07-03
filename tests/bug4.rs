// `crate_in_macro_def`: decycle's macro-carrier ping-pong re-quotes `Unparse` verbatim
// inside a generated `macro_rules!`, which is what trips the lint below (on tokens
// generated from this file, not on an item an inner `#[allow]` could attach to) — but
// `TraitA` is defined in THIS (the trait-defining) crate, not in decycle, so `$crate`
// inside the carrier would resolve against whatever crate re-invokes it instead, the
// wrong target for a locally-defined trait. `crate::` here is intentional.
#![allow(unused, non_camel_case_types, clippy::crate_in_macro_def)]

pub trait TraitA<A> {
    type S;
}

#[decycle::decycle]
pub trait Unparse<A> {
    fn unparse<S: crate::TraitA<A, S = S>>(sink: &mut S);
    fn f(sink: impl crate::TraitA<A, S = A>);
}

#[decycle::decycle]
mod m {
    use super::*;
    pub struct ItemMod {}
    impl<__A> Unparse<__A> for ItemMod {
        fn unparse<B: crate::TraitA<__A, S = B>>(_: &mut B) {}
        // TODO: this syntax can be supported desugaeing the impl Trait in XxxRanked trait
        // definition
        fn f(_sink: impl TraitA<__A, S = __A>) {}
    }
    #[decycle]
    use super::Unparse;
}
