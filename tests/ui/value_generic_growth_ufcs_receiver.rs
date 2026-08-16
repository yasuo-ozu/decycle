//! The genuine growth the old argument mapping HID: a UFCS call made from a method that has a
//! receiver.
//!
//! `<B as Eval>::eval(&B, &mut src, depth - 1)` passes the receiver as `args[0]`, so its arguments
//! line up one-to-one with the callee's inputs. The scan instead started at
//! `base = usize::from(has_receiver)`, i.e. 1 — checking `&B` against the by-value parameter's slot
//! and `&mut src` against `depth`'s — so this file compiled past the pre-pass and blew up in
//! monomorphisation exactly as `growth.rs` exists to prevent. It is now rejected with the same
//! message as the receiver-less spelling in `value_generic_growth_ranked.rs`.
use decycle::decycle;

pub trait Src {
    fn tick(&mut self) -> u32;
}
impl Src for u32 {
    fn tick(&mut self) -> u32 {
        *self
    }
}
impl<T: Src + ?Sized> Src for &mut T {
    fn tick(&mut self) -> u32 {
        (**self).tick()
    }
}

#[decycle]
mod m {
    use super::Src;

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
        fn eval<S: Src>(&self, mut src: S, depth: u32) -> u32 {
            if depth == 0 {
                return src.tick();
            }
            <B as Eval>::eval(&B, &mut src, depth - 1)
        }
    }

    impl Eval for B
    where
        A: Eval,
    {
        fn eval<S: Src>(&self, mut src: S, depth: u32) -> u32 {
            if depth == 0 {
                return src.tick();
            }
            <A as Eval>::eval(&A, &mut src, depth - 1)
        }
    }
}

fn main() {}
