//! The three shapes `growth.rs` must NOT reject. Each has a by-value generic parameter on a cyclic
//! method — condition 2 of the check — and each is finite, so each must still compile.
//!
//! A false positive here is worse than the false negative the check exists to replace: it rejects a
//! working program.
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

// (1) The value is MOVED along unchanged, so `S` never grows: `moved::<S>` only ever calls
//     `moved::<S>`. One `&mut` is added at the *entry* (`main` below), never per level.
#[decycle]
mod moved_along {
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
            <B as Eval>::eval(src, depth - 1)
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
            <A as Eval>::eval(src, depth - 1)
        }
    }
}

// (2) A reference IS passed, but to a LOCAL of a fixed type rather than to the generic parameter, so
//     the callee is instantiated at `&mut u32` at every level and the set closes after one step.
//     This is what condition 4 (root-binding taint) protects.
//
//     The `: u32` annotations below are LOAD-BEARING, not decoration. Taint follows `let` chains
//     through method calls — it has to, because `let s = src.into_src();` is the real-world spelling of
//     the growth — and `seed.tick()` is such a call. The annotation is the escape hatch: it names a
//     type that mentions no method generic, so the local is untainted. Remove it and this file is
//     (correctly, by the check's own rule) rejected.
#[decycle]
mod ref_to_local {
    use super::Src;

    #[decycle]
    pub trait Eval {
        fn eval<S: Src>(seed: S, depth: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Eval for A
    where
        B: Eval,
    {
        fn eval<S: Src>(mut seed: S, depth: u32) -> u32 {
            let mut local: u32 = seed.tick();
            if depth == 0 {
                return local;
            }
            <B as Eval>::eval(&mut local, depth - 1)
        }
    }
    impl Eval for B
    where
        A: Eval,
    {
        fn eval<S: Src>(mut seed: S, depth: u32) -> u32 {
            let mut local: u32 = seed.tick();
            if depth == 0 {
                return local;
            }
            <A as Eval>::eval(&mut local, depth - 1)
        }
    }
}

// (3) The FIX the diagnostic recommends: the recursion point takes `&mut S` and reborrows, so `S` is a
//     genuine fixed point. The by-value entry point delegates to it exactly once.
#[decycle]
mod by_reference {
    use super::Src;

    #[decycle]
    pub trait Eval {
        fn eval_stream<S: Src>(src: &mut S, depth: u32) -> u32;
        fn eval<S: Src>(mut src: S, depth: u32) -> u32 {
            Self::eval_stream(&mut src, depth)
        }
    }

    pub struct A;
    pub struct B;

    impl Eval for A
    where
        B: Eval,
    {
        fn eval_stream<S: Src>(src: &mut S, depth: u32) -> u32 {
            if depth == 0 {
                return src.tick();
            }
            <B as Eval>::eval_stream(&mut *src, depth - 1)
        }
    }
    impl Eval for B
    where
        A: Eval,
    {
        fn eval_stream<S: Src>(src: &mut S, depth: u32) -> u32 {
            if depth == 0 {
                return src.tick();
            }
            <A as Eval>::eval_stream(&mut *src, depth - 1)
        }
    }
}

fn main() {
    let mut n = 7u32;
    assert_eq!(<moved_along::A as moved_along::Eval>::eval(&mut n, 3), 7);
    assert_eq!(<ref_to_local::A as ref_to_local::Eval>::eval(&mut n, 3), 7);
    assert_eq!(<by_reference::A as by_reference::Eval>::eval(&mut n, 40), 7);
}
