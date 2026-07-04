//! D4 golden test: the `&mut dyn Trait` erasure recipe. `emit_reentry_items`'s fn-pointer alias
//! needs a NAMEABLE `fn(...) -> ...` type; an already-erased trait-object parameter (`&mut (dyn
//! Stream + '_)`, syan's `&mut dyn ParseStream` shape) is exactly that — concrete, non-generic,
//! non-`impl Trait` — so it re-enters unbounded through one fixed `&mut dyn` boundary (no
//! `Dup<…>`-style stream-type tower growth) instead of tripping the D4 `abort!` (which only
//! fires for a return-position `impl Trait`, a DIFFERENT shape).
#![allow(dead_code)]

use decycle::decycle;

/// A tiny, self-contained stand-in for syan's `dyn ParseStream`: an object-safe trait erasing the
/// concrete stream type behind a `&mut dyn`.
pub trait Stream {
    fn advance(&mut self) -> Option<u8>;
}

#[decycle]
mod m {
    #[decycle]
    pub trait Ca {
        fn ca(&self, stream: &mut (dyn crate::Stream + '_), n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, stream: &mut (dyn crate::Stream + '_), n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, stream: &mut (dyn crate::Stream + '_), n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(stream, n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, stream: &mut (dyn crate::Stream + '_), n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(stream, n - 1) + 1
            }
        }
    }
}

struct NullStream;
impl Stream for NullStream {
    fn advance(&mut self) -> Option<u8> {
        None
    }
}

#[test]
fn dyn_stream_reentry_is_unbounded() {
    use m::Ca;
    let mut s = NullStream;
    assert_eq!(m::A.ca(&mut s, 2000), 2000);
}
