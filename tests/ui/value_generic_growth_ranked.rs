//! A cyclic method takes a generic parameter BY VALUE and reborrows it at the recursive call, so
//! `eval::<S>` calls `eval::<&mut S>` calls `eval::<&mut &mut S>` … The obligation cycle is breakable
//! and both engines break it; the INSTANTIATION cycle is not, because the growth is in the
//! monomorphisation. See `decycle-impl/src/growth.rs`.
//!
//! Without the pre-pass this compiles as far as codegen and then reports
//! `error[E0275]: overflow evaluating the requirement &mut u32: Src` against the blanket impl on
//! line 12 — an innocent bystander — with no mention of decycle, of `S`, or of the fix. (Drop the
//! blanket and it becomes `reached the recursion limit while instantiating`, which has no error code
//! and points only at the call.)
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
        fn eval<S: Src>(src: S, depth: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Eval for A
    where
        B: Eval,
    {
        fn eval<S: Src>(mut src: S, depth: u32) -> u32 {
            if depth == 0 {
                return src.tick();
            }
            <B as Eval>::eval(&mut src, depth - 1)
        }
    }

    impl Eval for B
    where
        A: Eval,
    {
        fn eval<S: Src>(mut src: S, depth: u32) -> u32 {
            if depth == 0 {
                return src.tick();
            }
            <A as Eval>::eval(&mut src, depth - 1)
        }
    }
}

fn main() {}
