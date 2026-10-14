//! A `#[cfg]` on a method of an announced trait is evaluated in the defining crate: a method that is
//! cfg'd out there does not reach the `#[decycle] use` site. In 0.5.3 the consuming crate evaluated
//! it, and a cfg'd-out method that the carried trait still listed failed with E0576 (`cannot find
//! method or associated constant `gone` in trait `super::super::Gen``).
#![allow(dead_code)]

mod defs {
    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub trait Gen<T: ::core::clone::Clone> {
        #[cfg(any())]
        fn gone(&self);
        fn g(&self, t: T) -> usize;
    }
}

#[decycle::decycle]
mod ranked {
    #[decycle]
    use crate::defs::Gen;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Gen<u8> for A
    where
        B: Gen<u8>,
    {
        fn g(&self, t: u8) -> usize {
            self.0.as_ref().map_or(t as usize, |b| b.g(t) + 1)
        }
    }

    impl Gen<u8> for B
    where
        A: Gen<u8>,
    {
        fn g(&self, t: u8) -> usize {
            self.0.as_ref().map_or(t as usize, |a| a.g(t) + 10)
        }
    }
}

#[test]
fn ranked_cfg_gated_method_of_an_announced_trait() {
    use defs::Gen as _;
    // B.g(1) = 1; A.g(1) = 1 + 1
    assert_eq!(ranked::A(Some(Box::new(ranked::B(None)))).g(1), 2);
}
