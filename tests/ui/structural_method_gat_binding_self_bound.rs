//! A method-level bound that mentions `Self` in the generics of an associated-type binding
//! (`F: Fam<Assoc<Self> = u8>`) is rejected like any other method-level bound mentioning `Self`
//! (`structural_method_generic_self_bound.rs`): the method is emitted on the terminator and, as the
//! body holder, for the natural type, so one `F` would have to satisfy the bound for both `Self`s.
//! The binding's generics used to be skipped by the check.
use decycle::decycle;

pub trait Fam {
    type Assoc<T: ?Sized>;
}

#[decycle(structural)]
mod m {
    use super::Fam;

    #[decycle]
    pub trait Tr {
        fn gat<F: Fam<Assoc<Self> = u8>>(&self, f: F) -> u8;
    }

    pub struct A;
    pub struct B;

    impl Tr for A
    where
        B: Tr,
    {
        fn gat<F: Fam<Assoc<Self> = u8>>(&self, _f: F) -> u8 {
            1
        }
    }

    impl Tr for B
    where
        A: Tr,
    {
        fn gat<F: Fam<Assoc<Self> = u8>>(&self, _f: F) -> u8 {
            2
        }
    }
}

fn main() {}
