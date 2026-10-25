//! An argument-position `impl Trait` that mentions `Self` is rejected wherever it sits in the
//! argument's type, a generic argument (`Option<impl Fn(&Self)>`) included, as at the top
//! (`structural_impl_fn_self_arg.rs`): its cast target `Option<impl Fn(&__Term)>` is unnameable.
//! Only references, pointers, arrays, slices and tuples used to be searched.
use decycle::decycle;

#[decycle(structural)]
mod m {
    #[decycle]
    pub trait Tr {
        fn visit(&self, f: Option<impl Fn(&Self) -> u32>) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Tr for A
    where
        B: Tr,
    {
        fn visit(&self, f: Option<impl Fn(&Self) -> u32>) -> u32 {
            f.map_or(0, |f| f(self))
        }
    }

    impl Tr for B
    where
        A: Tr,
    {
        fn visit(&self, f: Option<impl Fn(&Self) -> u32>) -> u32 {
            f.map_or(1, |f| f(self))
        }
    }
}

fn main() {}
