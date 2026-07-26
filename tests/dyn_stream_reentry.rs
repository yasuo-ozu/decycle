//! The `&mut dyn Trait` erasure case: a cross-trait cycle threading an ALREADY-erased stream
//! (`&mut (dyn Stream + '_)`, syan's `&mut dyn ParseStream` shape). Because the stream parameter is
//! concrete/non-generic, re-entry crosses one fixed `&mut dyn` boundary with no `Dup<…>` stream-type
//! tower growth. Run under BOTH algorithms.
#![allow(dead_code)]

mod common;

/// A tiny stand-in for syan's `dyn ParseStream`: an object-safe trait erasing the concrete stream.
pub trait Stream {
    fn advance(&mut self) -> Option<u8>;
}

dual_mod! {
    m {
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
                if n == 0 { 0 } else { B.cb(stream, n - 1) + 1 }
            }
        }
        impl Cb for B
        where
            A: Ca,
        {
            fn cb(&self, stream: &mut (dyn crate::Stream + '_), n: usize) -> usize {
                if n == 0 { 0 } else { A.ca(stream, n - 1) + 1 }
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
    on_both!(m, {
        let mut s = NullStream;
        assert_eq!(A.ca(&mut s, 2000), 2000);
    });
}
