//! A `use super::…;` written INSIDE an adopted impl's method body.
//!
//! `syn` models a `use` item as a `UseTree`, not a `Path`, so `LiftRelative` never saw it; and a
//! `use` cannot be re-rooted at the alias module anyway — a 2018-edition `use` path has to start
//! with `crate`, `self`, `super`, `::` or a crate name, so there is no spelling of
//! `__DecycleRelMod_m::__DecycleRelPath_0_m` that a `use` accepts.
//!
//! Once the impl is re-emitted inside the generated helper module the import silently binds the
//! processed module's own `helper`, so this is rejected rather than left to resolve wrongly. Moving
//! the import out to the module level (where `finalize` does not move it) keeps working.
use decycle::decycle;

pub fn helper() -> usize {
    100
}

#[decycle]
mod m {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> usize;
    }

    pub fn helper() -> usize {
        1
    }

    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl Tr for A
    where
        B: Tr,
    {
        fn f(&self) -> usize {
            use super::helper;
            helper() + self.0.f()
        }
    }

    impl Tr for B
    where
        A: Tr,
    {
        fn f(&self) -> usize {
            self.0.as_ref().map(|a| a.f()).unwrap_or(0)
        }
    }
}

fn main() {}
