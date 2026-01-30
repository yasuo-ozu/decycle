#![allow(dead_code, private_interfaces)]

#[decycle::decycle]
mod generic_loops {
    #[decycle]
    trait GenA<T> {
        fn lift(value: T) -> Self;
        fn pair(&self, other: &Self, pair: (usize, usize)) -> usize;
    }

    #[decycle]
    trait GenB<T> {
        fn scale(self, factor: usize) -> usize;
        fn describe(&self) -> &'static str;
    }

    #[derive(Clone)]
    struct Boxed<T> {
        value: T,
        next: Option<Box<Boxed<T>>>,
    }

    impl<T: Clone> GenA<T> for Boxed<T>
    where
        Boxed<T>: GenB<T>,
    {
        fn lift(value: T) -> Self {
            Self {
                value,
                next: None,
            }
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_generic_loop() {
            let a = Boxed::lift(1u32);
            let b = Boxed::lift(2u32);
            assert_eq!(a.pair(&b, (2, 3)), 5);
            assert_eq!(b.clone().scale(4), 4);
        }
    }
}

#[decycle::decycle]
mod trait_object_loops {
    trait Helper {
        fn value(&self) -> i32;
    }

    #[decycle]
    trait ObjA {
        fn helper(&self) -> &dyn Helper;
        fn eval(&self, seed: i32) -> i32;
    }

    #[decycle]
    trait ObjB {
        fn helper(&self) -> &dyn Helper;
        fn eval(&self, seed: i32) -> i32;
    }

    struct Left {
        value: i32,
    }

    struct Right {
        value: i32,
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_trait_object_loop() {
            let left = Left { value: 1 };
            assert!(left.eval(2) > 0);
            assert_eq!(ObjA::helper(&left).value(), 5);
        }
    }
}

#[decycle::decycle]
mod lifetime_loops {
    #[decycle]
    trait BorrowA<'a> {
        fn link(&'a self, other: &'a Self) -> &'a str;
    }

    #[decycle]
    trait BorrowB<'a> {
        fn link(&'a self, other: &'a Self) -> &'a str;
    }

    struct Holder<'a> {
        label: &'a str,
        peer: Option<Box<Holder<'a>>>,
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_lifetime_loop() {
            let holder = Holder {
                label: "alpha",
                peer: None,
            };
            let other = Holder {
                label: "beta",
                peer: None,
            };
            assert_eq!(BorrowA::link(&holder, &other), "alpha");
        }
    }
}

#[decycle::decycle]
mod const_generic_loops {
    #[decycle]
    trait ConstA {
        fn count(&self) -> usize;
    }

    #[decycle]
    trait ConstB {
        fn count(&self) -> usize;
    }

    struct ArrayHolder<const N: usize> {
        data: [u8; N],
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn test_const_generics_loop() {
            let holder = ArrayHolder::<4> { data: [0, 1, 2, 3] };
            assert_eq!(ConstA::count(&holder), 4);
        }
    }
}

#[decycle::decycle]
mod async_like_loops {
    use core::future::Future;
    use core::pin::Pin;

    #[decycle]
    trait AsyncishA {
        fn run<'a>(&'a self, input: i32) -> Pin<Box<dyn Future<Output = i32> + 'a>>;
    }

    #[decycle]
    trait AsyncishB {
        fn run<'a>(&'a self, input: i32) -> Pin<Box<dyn Future<Output = i32> + 'a>>;
    }

    struct WorkerA {
        value: i32,
    }

    struct WorkerB {
        value: i32,
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

    #[cfg(test)]
    mod tests {
        use super::*;
        use core::future::Future;
        use core::pin::Pin;
        use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

        fn block_on<F: Future>(mut fut: F) -> F::Output {
            fn noop_clone(_: *const ()) -> RawWaker {
                RawWaker::new(core::ptr::null(), &VTABLE)
            }
            fn noop(_: *const ()) {}
            static VTABLE: RawWakerVTable =
                RawWakerVTable::new(noop_clone, noop, noop, noop);

            let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) };
            let mut cx = Context::from_waker(&waker);
            let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
            loop {
                if let Poll::Ready(val) = fut.as_mut().poll(&mut cx) {
                    return val;
                }
            }
        }

        #[test]
        fn test_async_like_loop() {
            let a = WorkerA { value: 2 };
            let b = WorkerB { value: 3 };
            let a_val = block_on(a.run(4));
            let b_val = block_on(b.run(5));
            assert_eq!(a_val, 6);
            assert_eq!(b_val, 8);
        }
    }
}
