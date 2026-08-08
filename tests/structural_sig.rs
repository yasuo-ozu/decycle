//! Structural-engine signature-rewriting regressions.
//!
//! Both of these are completeness gaps in how a method signature is carried into terminator space,
//! and both are structural-only — the ranked engine already handled them.
#![allow(dead_code)]

use decycle::decycle;

#[decycle(structural)]
mod sig {
    #[decycle]
    pub trait Tr {
        type Out;
        type Key;
        /// A parameter typed through a `Self` projection. `mentions_self` only looked at the LAST
        /// path segment, so `Self::Out` was never recognised as Self-mentioning and was forwarded
        /// without the terminator cast -> `error[E0308]: expected A, found __ATerm_<nonce>`.
        fn seed(&self, s: Self::Out) -> i64;
        /// A projection that does NOT resolve to `Self` must still be forwarded untouched — the
        /// fix must key on what the projection resolves to, not on the spelling.
        fn plain(&self, k: Self::Key) -> i64;
        /// Impls may bind the receiver `mut self`. `normalize_sig` rewrote only typed params, so
        /// the `mut` reached the bodiless `__run` decl -> "patterns aren't allowed in functions
        /// without bodies" (a deny-by-default future-compat error).
        fn take(self) -> i64;
    }
    pub struct A(pub i64, pub Box<B>);
    pub struct B(pub i64);

    impl Tr for A
    where
        B: Tr,
    {
        type Out = Self;
        type Key = i64;
        fn seed(&self, s: Self::Out) -> i64 {
            self.0 + s.0
        }
        fn plain(&self, k: Self::Key) -> i64 {
            self.0 + k
        }
        fn take(mut self) -> i64 {
            self.0 += 5;
            self.0
        }
    }
    impl Tr for B
    where
        A: Tr,
    {
        type Out = Self;
        type Key = i64;
        fn seed(&self, s: Self::Out) -> i64 {
            self.0 + s.0
        }
        fn plain(&self, k: Self::Key) -> i64 {
            self.0 + k
        }
        fn take(mut self) -> i64 {
            self.0 += 5;
            self.0
        }
    }
}

#[test]
fn self_projection_param_is_cast() {
    use sig::Tr;
    assert_eq!(sig::B(1).seed(sig::B(2)), 3);
    assert_eq!(sig::A(1, Box::new(sig::B(0))).seed(sig::A(2, Box::new(sig::B(0)))), 3);
}

#[test]
fn non_self_projection_param_is_forwarded_untouched() {
    use sig::Tr;
    assert_eq!(sig::B(1).plain(41), 42);
}

#[test]
fn mut_self_receiver_compiles_and_mutates() {
    use sig::Tr;
    // Asserts the MUTATION, not merely that it compiles: the `mut` has to survive onto the copy
    // that holds the user's body while staying off the bodiless decl.
    assert_eq!(sig::B(10).take(), 15);
    assert_eq!(sig::A(10, Box::new(sig::B(0))).take(), 15);
}
