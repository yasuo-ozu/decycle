//! The growth check must not fire on a call that is NOT the cycle's own trait method.
//!
//! `growth.rs` matched a call site purely on the METHOD NAME and mapped its arguments by position
//! in the CALLER's signature. It never checked that the callee was the same trait method, that the
//! callee's parameter at that position was taken by value, or even that the call was a trait method
//! call at all — so `Helper.eval(&src)`, where `Aux::eval<S>(&self, src: &S)` is a *different*
//! trait's method taking `S` **by reference** (no growth whatsoever: `S` is the same at every
//! level), was rejected with the confident and factually wrong
//!
//!     decycle: this recursive call reborrows `eval`'s own by-value generic parameter …
//!
//! There is no escape hatch for it, and the identical program compiles and runs without
//! `#[decycle]`.
//!
//! Two more shapes here that must stay accepted, for the same reason:
//!
//! * a FREE function `eval(&src)` that happens to share the method's name — a bare single-segment
//!   path is never a trait method;
//! * a UFCS call from a method WITH a receiver (`<B as Eval>::eval(&b, src, ..)`), whose arguments
//!   used to be mis-mapped by one (`base = usize::from(has_receiver)`), so the *receiver* was
//!   checked against the by-value parameter's position. (The true positive that same off-by-one
//!   hid is pinned by `tests/ui/value_generic_growth_ufcs_receiver.rs`.)
use decycle::decycle;

pub trait Src {
    fn tick(&self) -> u32;
}
impl Src for u32 {
    fn tick(&self) -> u32 {
        *self
    }
}

/// A different trait, whose same-named method takes `S` BY REFERENCE.
pub trait Aux {
    fn eval<S: Src>(&self, src: &S) -> u32;
}
pub struct Helper;
impl Aux for Helper {
    fn eval<S: Src>(&self, src: &S) -> u32 {
        src.tick()
    }
}

/// A free function sharing the method's name, also taking its generic by reference.
pub fn eval<S: Src>(src: &S) -> u32 {
    src.tick() + 1
}

#[decycle]
mod m {
    // Imported unrenamed, so the call below is spelled with the METHOD's own name.
    use super::{eval, Aux, Helper, Src};

    #[decycle]
    pub trait Eval {
        fn eval<S: Src>(&self, src: S, depth: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Eval for A
    where
        B: Eval,
    {
        fn eval<S: Src>(&self, src: S, depth: u32) -> u32 {
            if depth == 0 {
                // A DIFFERENT trait's method, taking `S` by reference: no growth.
                return Helper.eval(&src);
            }
            // UFCS with a receiver — `&B` is `args[0]`, `src` is `args[1]`.
            <B as Eval>::eval(&B, src, depth - 1)
        }
    }

    impl Eval for B
    where
        A: Eval,
    {
        fn eval<S: Src>(&self, src: S, depth: u32) -> u32 {
            if depth == 0 {
                // A free function sharing the name, also by reference: no growth.
                return eval(&src);
            }
            A.eval(src, depth - 1)
        }
    }
}

fn main() {
    use m::Eval as _;
    assert_eq!(m::A.eval(7u32, 0), 7);
    assert_eq!(m::A.eval(7u32, 1), 8);
    assert_eq!(m::A.eval(7u32, 4), 7);
    assert_eq!(m::B.eval(7u32, 2), 8);
}
