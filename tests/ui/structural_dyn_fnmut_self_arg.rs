//! A `&dyn FnMut(&Self)` argument cannot be forwarded: the rebuilt-by-coercion adapter only covers
//! `&dyn Fn(..)`, and punning the wide pointer would leave a vtable naming a trait that does not
//! exist once `Self` is substituted.
//!
//! This pins the ADVICE as much as the rejection. The message used to recommend "use a generic
//! parameter (`F: FnMut(&Self)`)" — which cannot compile under this engine at all (a method-level
//! generic bound mentioning `Self` is now rejected outright,
//! `tests/ui/structural_method_generic_self_bound.rs`), so the suggestion sent the reader from one
//! error to another. It now names only forms that work here, plus the other engine.
use decycle::decycle;

#[decycle(structural)]
mod m {
    #[decycle]
    pub trait Vis {
        fn apply(&self, f: &dyn FnMut(&Self)) -> i64;
    }
    pub struct A(pub i64);
    impl Vis for A
    where
        A: Vis,
    {
        fn apply(&self, _f: &dyn FnMut(&Self)) -> i64 {
            self.0
        }
    }
}

fn main() {}
