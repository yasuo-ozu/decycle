//! A `super::`-rooted path inside a macro invocation, in an adopted impl body.
//!
//! `syn` hands a macro invocation over as an opaque token stream, so `LiftRelative` cannot rewrite
//! the path the way it rewrites every other `super::` occurrence — and rewriting arbitrary macro
//! input is not sound in general (`stringify!` and friends read idents as data). Left alone the
//! path still RESOLVES once the impl is re-emitted inside the generated helper module: to the
//! processed module's own `helper`, silently returning 1 where 100 is correct.
//!
//! Silent-wrong is the thing to eliminate, so this is rejected instead, at the macro the caller
//! wrote. The same path outside a macro is lifted and keeps working — see
//! `tests/ranked_support_regressions.rs::super_value_paths`.
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
            assert_eq!(super::helper(), 100);
            self.0.f()
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
