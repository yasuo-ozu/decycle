#![allow(dead_code, private_interfaces)]

#[decycle::decycle(support_infinite_cycle = true)]
mod advanced_cycles_infinite {
    #[decycle]
    trait CycleA {
        const KIND: usize;

        fn build(value: i32) -> i32;
        fn consume(self) -> i32;
        fn update(&mut self, pair: (i32, i32)) -> i32;
        fn inspect(&self) -> usize;
        fn boxed(self: Box<Self>) -> i32;
    }

    #[decycle]
    trait CycleB {
        const OFFSET: i32;

        fn make(flag: bool) -> Vec<i32>;
        fn compute(&self, pair: (i32, i32)) -> i32;
        fn into_value(self) -> i32;
    }

    trait AssocLink {
        type Out;
        fn wrap(&self) -> Self::Out;
    }

    #[derive(Clone, Debug)]
    struct NodeA {
        value: i32,
        child: Option<Box<NodeB>>,
    }

    #[derive(Clone, Debug)]
    struct NodeB {
        value: i32,
        child: Option<Box<NodeA>>,
    }

    impl NodeA {
        const KIND_VALUE: usize = 1;
    }

    impl NodeB {
        const OFFSET_VALUE: i32 = 7;
    }

    impl AssocLink for NodeA {
        type Out = (i32, String);

        fn wrap(&self) -> Self::Out {
            (self.value, format!("A:{}", self.value))
        }
    }

    impl AssocLink for NodeB {
        type Out = (i32, String);

        fn wrap(&self) -> Self::Out {
            (self.value, format!("B:{}", self.value))
        }
    }

    impl CycleA for NodeA
    where
        NodeB: CycleB,
    {
        const KIND: usize = 1;

        fn build(value: i32) -> i32 {
            value + 1
        }

        fn consume(self) -> i32 {
            self.value + self.child.as_ref().map_or(0, |b| b.compute((1, 2)))
        }

        fn update(&mut self, (lhs, rhs): (i32, i32)) -> i32 {
            self.value += lhs + rhs;
            self.value
        }

        fn inspect(&self) -> usize {
            self.child.as_ref().map_or(0, |_| 1) + NodeA::KIND_VALUE
        }

        fn boxed(self: Box<Self>) -> i32 {
            self.value
        }
    }

    impl CycleB for NodeB
    where
        NodeA: CycleA,
    {
        const OFFSET: i32 = 7;

        fn make(flag: bool) -> Vec<i32> {
            if flag { vec![1, 2, 3] } else { vec![4] }
        }

        fn compute(&self, (a, b): (i32, i32)) -> i32 {
            self.value + a + b + NodeB::OFFSET_VALUE
        }

        fn into_value(self) -> i32 {
            self.value + NodeA::KIND_VALUE as i32
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use super::{CycleA, CycleB};

        #[test]
        fn test_associated_consts_and_receivers_infinite() {
            let mut a = NodeA {
                value: 3,
                child: None,
            };

            let value = NodeA::build(10);
            assert_eq!(value, 11);
            assert_eq!(NodeA::KIND_VALUE, 1);

            a.update((2, 4));
            assert!(a.inspect() >= 1);
            let boxed = Box::new(a).boxed();
            assert!(boxed >= 3);
        }

        #[test]
        fn test_assoc_type_and_patterns_infinite() {
            let node_a = NodeA {
                value: 1,
                child: Some(Box::new(NodeB { value: 2, child: None })),
            };
            let node_b = NodeB { value: 5, child: None };

            let wrapped = node_a.wrap();
            assert!(wrapped.1.contains('A'));
            assert!(node_a.clone().consume() >= 1);
            assert_eq!(node_b.clone().into_value(), 6);
            let _items = <NodeB as CycleB>::make(true);
        }
    }
}

#[decycle::decycle(support_infinite_cycle = false)]
mod advanced_cycles_finite {
    #[decycle]
    trait GammaLoop {
        const LIMIT: usize;

        fn create(seed: i32) -> (i32, i32);
        fn eval(&self) -> i32;
        fn set_all(&mut self, triple: (i32, i32, i32));
    }

    #[decycle]
    trait DeltaLoop {
        const SCALE: i32;

        fn new(seed: i32) -> Option<i32>;
        fn total(&self) -> i32;
        fn take(self) -> i32;
    }

    trait AssocState {
        type State;
        fn state(&self) -> Self::State;
    }

    struct Left {
        value: i32,
        right: Option<Box<Right>>,
    }

    struct Right {
        value: i32,
        left: Option<Box<Left>>,
    }

    impl Left {
        const LIMIT_VALUE: usize = 3;
    }

    impl Right {
        const SCALE_VALUE: i32 = 2;
    }

    impl AssocState for Left {
        type State = (i32, bool);

        fn state(&self) -> Self::State {
            (self.value, self.value % 2 == 0)
        }
    }

    impl AssocState for Right {
        type State = (i32, bool);

        fn state(&self) -> Self::State {
            (self.value, self.value % 2 == 1)
        }
    }

    impl GammaLoop for Left
    where
        Right: DeltaLoop,
    {
        const LIMIT: usize = 3;

        fn create(seed: i32) -> (i32, i32) {
            (seed, seed + 1)
        }

        fn eval(&self) -> i32 {
            let bonus = self.right.as_ref().map_or(0, |r| r.total());
            self.value + bonus
        }

        fn set_all(&mut self, (x, y, z): (i32, i32, i32)) {
            self.value = x + y + z;
        }
    }

    impl DeltaLoop for Right
    where
        Left: GammaLoop,
    {
        const SCALE: i32 = 2;

        fn new(seed: i32) -> Option<i32> {
            Some(seed * Right::SCALE_VALUE)
        }

        fn total(&self) -> i32 {
            self.value * Right::SCALE_VALUE + self.left.as_ref().map_or(0, |l| l.eval())
        }

        fn take(self) -> i32 {
            self.value
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use super::{DeltaLoop, GammaLoop};

        #[test]
        fn test_associated_types_and_consts_finite() {
            let left = Left {
                value: 5,
                right: None,
            };

            let right = Right {
                value: 2,
                left: Some(Box::new(left)),
            };

            let data = Right::new(4);
            assert!(data.is_some());
            assert!(right.total() >= 2);
            let _ = Left::LIMIT_VALUE;
            let _pair = <Left as GammaLoop>::create(9);
            let _state = right.state();
        }
    }
}
