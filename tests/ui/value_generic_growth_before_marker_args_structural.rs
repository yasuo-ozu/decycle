//! A module with both a value-generic growth and a marker with arguments reports the growth. The
//! growth pre-pass runs before the structural engine takes the markers, and it no longer checks
//! them on its way: their `takes no arguments` is the engine's to report, after it. It was reported
//! first, and the growth (`value_generic_growth_structural.rs`) not at all.
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

#[decycle(structural)]
mod m {
    use super::Src;

    #[decycle(recurse_level = 3)]
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
