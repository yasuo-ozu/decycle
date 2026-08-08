//! An argument-position `impl Trait` that mentions `Self` (`impl Fn(&Self)`) can't be cast into
//! terminator-space by the structural engine — its cast target `impl Fn(&__Term)` is unnameable
//! (E0562), unlike a `fn(&Self)` / `&dyn Fn(&Self)` arg (both of which ARE supported). Rejected
//! up-front, on the user's own parameter, pointing at the concrete forms. (The ranked engine
//! supports all three.)
use decycle::decycle;

#[decycle(structural)]
mod m {
    #[decycle]
    pub trait Tr {
        fn apply(&self, f: impl Fn(&Self) -> i64) -> i64;
    }
    pub struct A(pub i64);
    impl Tr for A
    where
        A: Tr,
    {
        fn apply(&self, f: impl Fn(&Self) -> i64) -> i64 {
            f(self)
        }
    }
}

fn main() {}
