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
