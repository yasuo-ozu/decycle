#![allow(unused, non_camel_case_types)]

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
