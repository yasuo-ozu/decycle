#[decycle::decycle]
mod integration_circular {
    #[decycle]
    pub trait TestTrait {
        fn test_method(&self) -> String;
    }

    #[decycle]
    pub trait LocalTrait {
        fn local_method(&self) -> usize;
    }

    pub struct NodeA {
        pub name: String,
        pub child_b: Option<Box<NodeB>>,
    }

    pub struct NodeB {
        pub count: usize,
        pub child_a: Option<Box<NodeA>>,
    }

    impl TestTrait for NodeA
    where
        NodeB: LocalTrait,
    {
        fn test_method(&self) -> String {
            let child_count = self.child_b.as_ref().map_or(0, |b| b.local_method());
            format!("NodeA:{}:{}", self.name, child_count)
        }
    }

    impl LocalTrait for NodeB
    where
        NodeA: TestTrait,
    {
        fn local_method(&self) -> usize {
            let child_len = self
                .child_a
                .as_ref()
                .map_or(0, |a| a.test_method().len());
            self.count + child_len
        }
    }
}

use integration_circular::*;
use integration_circular::{LocalTrait, TestTrait};

#[test]
fn test_circular_coinduction_minimal() {
    let node_a = NodeA {
        name: "alpha".to_string(),
        child_b: None,
    };

    let node_b = NodeB {
        count: 3,
        child_a: Some(Box::new(node_a)),
    };

    let node_a2 = NodeA {
        name: "beta".to_string(),
        child_b: Some(Box::new(node_b)),
    };

    assert!(node_a2.test_method().contains("NodeA:beta"));
}

#[test]
fn test_circular_coinduction_sizes() {
    let node_a = NodeA {
        name: "gamma".to_string(),
        child_b: None,
    };

    let node_b = NodeB {
        count: 5,
        child_a: Some(Box::new(node_a)),
    };

    assert!(node_b.local_method() >= 5);
}
