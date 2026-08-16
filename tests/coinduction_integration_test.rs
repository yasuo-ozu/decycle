//! Cross-trait cycle where one trait (`LocalTrait`) is imported from ANOTHER crate via
//! `#[decycle] pub use`: `NodeA: TestTrait → NodeB: LocalTrait → NodeA: TestTrait`. Run under BOTH
//! algorithms (structural exercises the `(type, trait)`-pair graph across an external trait).
mod common;

dual_mod! {
    integration_circular {
        #[decycle]
        pub trait TestTrait {
            fn test_method(&self) -> String;
        }

        #[decycle]
        pub use traitdef::LocalTrait;

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
                let child_len = self.child_a.as_ref().map_or(0, |a| a.test_method().len());
                self.count + child_len
            }
        }
    }
}

#[test]
fn test_circular_coinduction_minimal() {
    on_both!(integration_circular, {
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
        // Exact, not `contains`: the recursive part is the `:16` suffix, so a `contains("NodeA:beta")`
        // assertion passes even when the cycle contributes nothing at all.
        assert_eq!(node_a2.test_method(), "NodeA:beta:16");
    });
}

#[test]
fn test_circular_coinduction_sizes() {
    on_both!(integration_circular, {
        let node_a = NodeA {
            name: "gamma".to_string(),
            child_b: None,
        };
        let node_b = NodeB {
            count: 5,
            child_a: Some(Box::new(node_a)),
        };
        // Exact: `count` alone is 5, so `>= 5` held even if the cross-crate cycle never recursed.
        assert_eq!(node_b.local_method(), 18);
    });
}
