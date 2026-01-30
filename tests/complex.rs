#![allow(dead_code)]

#[decycle::decycle]
mod complex_system {
    #[decycle]
    pub trait TraitA {
        fn get_a(&self) -> usize;
    }

    #[decycle]
    pub trait TraitB {
        fn get_b(&self) -> usize;
    }

    pub struct NodeA {
        pub value: usize,
        pub child_b: Option<Box<NodeB>>,
    }

    pub struct NodeB {
        pub value: usize,
        pub child_a: Option<Box<NodeA>>,
    }

    impl TraitA for NodeA
    where
        NodeB: TraitB,
    {
        fn get_a(&self) -> usize {
            let child = self.child_b.as_ref().map_or(0, |b| b.get_b());
            self.value + child
        }
    }

    impl TraitB for NodeB
    where
        NodeA: TraitA,
    {
        fn get_b(&self) -> usize {
            let child = self.child_a.as_ref().map_or(0, |a| a.get_a());
            self.value + child
        }
    }
}

use complex_system::*;

#[cfg(test)]
mod tests {
    use super::*;
    use complex_system::{TraitA, TraitB};

    #[test]
    fn test_simple_cycle() {
        let node_a = NodeA {
            value: 1,
            child_b: None,
        };

        let node_b = NodeB {
            value: 2,
            child_a: Some(Box::new(node_a)),
        };

        let node_a2 = NodeA {
            value: 3,
            child_b: Some(Box::new(node_b)),
        };

        assert!(node_a2.get_a() >= 3);
    }

    #[test]
    fn test_nested_cycle() {
        let node_a = NodeA {
            value: 4,
            child_b: None,
        };

        let node_b = NodeB {
            value: 5,
            child_a: Some(Box::new(node_a)),
        };

        assert!(node_b.get_b() >= 5);
    }
}
