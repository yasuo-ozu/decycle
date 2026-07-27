//! A diverse set of cross-trait cycles — generic traits (`-> Self`, `&Self`, destructured params,
//! `Self::` ctor), `&dyn`-trait returns, trait lifetime generics (`&'a self`), const generics on the
//! self type, and boxed-future (async) returns. Every module runs under BOTH algorithms.
#![allow(dead_code)]
mod common;

// ---- generic traits: `-> Self`, `&Self` arg, destructured param, `Self::` ctor ----
dual_mod! {
    generic_loops {
        #[decycle]
        pub trait GenA<T> {
            fn lift(value: T) -> Self;
            fn pair(&self, other: &Self, pair: (usize, usize)) -> usize;
        }
        #[decycle]
        pub trait GenB<T> {
            fn scale(self, factor: usize) -> usize;
            fn describe(&self) -> &'static str;
        }
        #[derive(Clone)]
        pub struct Boxed<T> {
            pub value: T,
            pub next: Option<Box<Boxed<T>>>,
        }
        impl<T: Clone> GenA<T> for Boxed<T>
        where
            Boxed<T>: GenB<T>,
        {
            fn lift(value: T) -> Self {
                Self { value, next: None }
            }
            fn pair(&self, other: &Self, (x, y): (usize, usize)) -> usize {
                let _ = other.describe();
                x + y + if self.next.is_some() { 1 } else { 0 }
            }
        }
        impl<T: Clone> GenB<T> for Boxed<T>
        where
            Boxed<T>: GenA<T>,
        {
            fn scale(self, factor: usize) -> usize {
                let _ = Self::lift(self.value.clone());
                factor
            }
            fn describe(&self) -> &'static str {
                "boxed"
            }
        }
    }
}

#[test]
fn test_generic_loop() {
    on_both!(generic_loops, {
        let a: Boxed<u32> = Boxed::lift(1);
        let b: Boxed<u32> = Boxed::lift(2);
        assert_eq!(a.pair(&b, (2, 3)), 5);
        assert_eq!(b.clone().scale(4), 4);
    });
}

// ---- `&dyn`-trait return; obligation cycle with non-recursive bodies ----
dual_mod! {
    trait_object_loops {
        pub trait Helper {
            fn value(&self) -> i32;
        }
        #[decycle]
        pub trait ObjA {
            fn helper(&self) -> &dyn Helper;
            fn eval(&self, seed: i32) -> i32;
        }
        #[decycle]
        pub trait ObjB {
            fn helper(&self) -> &dyn Helper;
            fn eval(&self, seed: i32) -> i32;
        }
        pub struct Left {
            pub value: i32,
        }
        pub struct Right {
            pub value: i32,
        }
        impl Helper for Left {
            fn value(&self) -> i32 {
                self.value
            }
        }
        impl Helper for Right {
            fn value(&self) -> i32 {
                self.value
            }
        }
        impl ObjA for Left
        where
            Right: ObjB,
        {
            fn helper(&self) -> &dyn Helper {
                static RIGHT: Right = Right { value: 5 };
                &RIGHT
            }
            fn eval(&self, seed: i32) -> i32 {
                self.value + seed
            }
        }
        impl ObjB for Right
        where
            Left: ObjA,
        {
            fn helper(&self) -> &dyn Helper {
                static LEFT: Left = Left { value: 3 };
                &LEFT
            }
            fn eval(&self, seed: i32) -> i32 {
                self.value + seed
            }
        }
    }
}

#[test]
fn test_trait_object_loop() {
    on_both!(trait_object_loops, {
        let left = Left { value: 1 };
        assert!(left.eval(2) > 0);
        assert_eq!(ObjA::helper(&left).value(), 5);
    });
}

// ---- trait lifetime generic; `&'a self` receiver + `&'a Self` arg ----
dual_mod! {
    lifetime_loops {
        #[decycle]
        pub trait BorrowA<'a> {
            fn link(&'a self, other: &'a Self) -> &'a str;
        }
        #[decycle]
        pub trait BorrowB<'a> {
            fn link(&'a self, other: &'a Self) -> &'a str;
        }
        pub struct Holder<'a> {
            pub label: &'a str,
            pub peer: Option<Box<Holder<'a>>>,
        }
        impl<'a> BorrowA<'a> for Holder<'a>
        where
            Holder<'a>: BorrowB<'a>,
        {
            fn link(&'a self, other: &'a Self) -> &'a str {
                let _ = other.peer.as_ref();
                self.label
            }
        }
        impl<'a> BorrowB<'a> for Holder<'a>
        where
            Holder<'a>: BorrowA<'a>,
        {
            fn link(&'a self, other: &'a Self) -> &'a str {
                let _ = other.peer.as_ref();
                self.label
            }
        }
    }
}

#[test]
fn test_lifetime_loop() {
    on_both!(lifetime_loops, {
        let holder = Holder {
            label: "alpha",
            peer: None,
        };
        let other = Holder {
            label: "beta",
            peer: None,
        };
        assert_eq!(BorrowA::link(&holder, &other), "alpha");
    });
}

// ---- const generic on the self type ----
dual_mod! {
    const_generic_loops {
        #[decycle]
        pub trait ConstA {
            fn count(&self) -> usize;
        }
        #[decycle]
        pub trait ConstB {
            fn count(&self) -> usize;
        }
        pub struct ArrayHolder<const N: usize> {
            pub data: [u8; N],
        }
        impl ConstA for ArrayHolder<4>
        where
            ArrayHolder<4>: ConstB,
        {
            fn count(&self) -> usize {
                self.data.len()
            }
        }
        impl ConstB for ArrayHolder<4>
        where
            ArrayHolder<4>: ConstA,
        {
            fn count(&self) -> usize {
                self.data.len()
            }
        }
    }
}

#[test]
fn test_const_generics_loop() {
    on_both!(const_generic_loops, {
        let holder = ArrayHolder::<4> { data: [0, 1, 2, 3] };
        assert_eq!(ConstA::count(&holder), 4);
    });
}

// ---- method lifetime generic + boxed-future (async) return ----
dual_mod! {
    async_like_loops {
        use core::future::Future;
        use core::pin::Pin;
        #[decycle]
        pub trait AsyncishA {
            fn run<'a>(&'a self, input: i32) -> Pin<Box<dyn Future<Output = i32> + 'a>>;
        }
        #[decycle]
        pub trait AsyncishB {
            fn run<'a>(&'a self, input: i32) -> Pin<Box<dyn Future<Output = i32> + 'a>>;
        }
        pub struct WorkerA {
            pub value: i32,
        }
        pub struct WorkerB {
            pub value: i32,
        }
        impl AsyncishA for WorkerA
        where
            WorkerB: AsyncishB,
        {
            fn run<'a>(&'a self, input: i32) -> Pin<Box<dyn Future<Output = i32> + 'a>> {
                Box::pin(async move { self.value + input })
            }
        }
        impl AsyncishB for WorkerB
        where
            WorkerA: AsyncishA,
        {
            fn run<'a>(&'a self, input: i32) -> Pin<Box<dyn Future<Output = i32> + 'a>> {
                Box::pin(async move { self.value + input })
            }
        }
    }
}

fn block_on<F: core::future::Future>(mut fut: F) -> F::Output {
    use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop_clone(_: *const ()) -> RawWaker {
        RawWaker::new(core::ptr::null(), &VTABLE)
    }
    fn noop(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(noop_clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = unsafe { core::pin::Pin::new_unchecked(&mut fut) };
    loop {
        if let Poll::Ready(val) = fut.as_mut().poll(&mut cx) {
            return val;
        }
    }
}

#[test]
fn test_async_like_loop() {
    on_both!(async_like_loops, {
        let a = WorkerA { value: 2 };
        let b = WorkerB { value: 3 };
        assert_eq!(block_on(a.run(4)), 6);
        assert_eq!(block_on(b.run(5)), 8);
    });
}
