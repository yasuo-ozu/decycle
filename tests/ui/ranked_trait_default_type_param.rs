//! A `#[decycle]` trait may not declare a DEFAULT for one of its own generic parameters.
//!
//! Every use site's written argument list has to match the declaration one-for-one (decycle only
//! inserts the synthesized rank into whatever the user wrote), so an omitted defaulted argument —
//! `impl Ca for A` below — used to expand to a rank-lowered path with too few arguments and
//! produce an E0107 cascade naming decycle's own generated internals (`Rank8909…`, the `__Mk`
//! marker, the `__Fp` alias). Rejected up front instead, with the one-line fix in the message.
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Ca<T = u8> {
        fn ca(&self, t: T, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, _t: u8, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(0u8, n - 1) + 1
            }
        }
    }
}

fn main() {}
