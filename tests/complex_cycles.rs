//! F-CC: the RELAXED 6-trait dense-cycle port of the long-disabled `complex_coinduction` block
//! (`tests/complex_coinduction.rs`, commented out since the pre-`#[decycle]` `#[coinduction]`
//! API, with the comment "requires recursion limit increases … beyond the current
//! implementation's capabilities"). That comment was wrong even before this branch — the port
//! below (minus the block's pruned `Vec`-typed calls) compiles and runs fine as an ordinary
//! `#[decycle]` module, in BOTH modes. No method body here recurses across the cycle at all
//! (the cyclic bounds are type-level only, matching the `hetero_side_bounds` shape in
//! `unbounded_reentry.rs`), so this is a pure "does it compile and run shallowly" test — it is
//! F-C1's side-bound reachability check, exercised at width 6 with heavily divergent
//! `Send`/`Sync`/`Debug`/`Default`/`PartialEq`/`Clone` side-bounds across the impls, that makes
//! it compile at all in default (unbounded) mode.

use decycle::decycle;

#[decycle]
trait Compute<T> {
    type Output;
    fn compute(&self, input: T) -> Self::Output;
}

#[decycle]
trait Process {
    fn process(&self);
}

#[decycle]
trait Validate<T> {
    fn validate(&self, data: T) -> bool;
}

#[decycle]
trait Transform<From, To> {
    fn transform(&self, from: From) -> To;
}

#[decycle]
trait Cache<K, V> {
    fn get(&self, key: K) -> ::core::option::Option<V>;
    fn put(&self, key: K, value: V);
}

#[decycle]
trait Serialize<T> {
    fn serialize(&self, data: T) -> ::std::vec::Vec<u8>;
}

// ---- ordinary types implementing the traits directly (outside any decycle module) ----
mod compute_types {
    pub struct FastProcessor<T>(pub T);
    pub struct SlowProcessor<T>(pub T);

    impl<T: Clone> super::Compute<T> for FastProcessor<T> {
        type Output = T;
        fn compute(&self, input: T) -> Self::Output {
            input
        }
    }
    impl<T: Clone> super::Process for FastProcessor<T> {
        fn process(&self) {}
    }
    impl<T: Clone + Default> super::Compute<T> for SlowProcessor<T> {
        type Output = T;
        fn compute(&self, input: T) -> Self::Output {
            input
        }
    }
    impl<T: Clone + Default> super::Process for SlowProcessor<T> {
        fn process(&self) {}
    }
}

mod storage_types {
    use std::collections::HashMap;

    pub struct MemoryCache<K, V> {
        pub data: HashMap<K, V>,
    }
    pub struct DiskCache<K, V> {
        // Never read: this struct is only used as a type-level shape in cyclic
        // bounds/trait impls (compile-and-run-shallowly test), never instantiated.
        #[allow(dead_code)]
        pub path: String,
        pub _phantom: std::marker::PhantomData<(K, V)>,
    }

    impl<K: Clone + Eq + std::hash::Hash, V: Clone> super::Cache<K, V> for MemoryCache<K, V> {
        fn get(&self, key: K) -> Option<V> {
            self.data.get(&key).cloned()
        }
        fn put(&self, _key: K, _value: V) {}
    }
    impl<K: Clone, V: Clone> super::Serialize<(K, V)> for MemoryCache<K, V> {
        fn serialize(&self, _data: (K, V)) -> Vec<u8> {
            vec![1, 2, 3]
        }
    }
    impl<K: Clone, V: Clone> super::Cache<K, V> for DiskCache<K, V> {
        fn get(&self, _key: K) -> Option<V> {
            None
        }
        fn put(&self, _key: K, _value: V) {}
    }
    impl<K, V> super::Serialize<(K, V)> for DiskCache<K, V> {
        fn serialize(&self, _data: (K, V)) -> Vec<u8> {
            vec![4, 5, 6]
        }
    }
}

mod validation_types {
    pub struct EmailValidator;
    pub struct NumberValidator<T>(pub T);
    pub struct DataTransformer<T>(pub T);

    impl super::Validate<::std::string::String> for EmailValidator {
        fn validate(&self, data: String) -> bool {
            data.contains('@')
        }
    }
    impl super::Transform<::std::string::String, ::core::primitive::bool> for EmailValidator {
        fn transform(&self, from: String) -> bool {
            from.len() > 5
        }
    }
    impl<T: PartialOrd + Clone> super::Validate<T> for NumberValidator<T> {
        fn validate(&self, data: T) -> bool {
            data >= self.0
        }
    }
    impl<T: Clone + std::fmt::Display> super::Transform<T, String> for DataTransformer<T> {
        fn transform(&self, from: T) -> String {
            format!("{}", from)
        }
    }
}

// Shared macro for the two mode variants (default/unbounded and `support_infinite_cycle =
// false`/bounded) — only the module's own `#[decycle(...)]` attribute and module name differ,
// so the dense cycle body is written once.
macro_rules! dense_cycle_module {
    ($decycle_attr:meta, $mod_name:ident) => {
        #[$decycle_attr]
        mod $mod_name {
            #[decycle]
            use super::{Cache, Compute, Process, Serialize, Transform, Validate};
            use super::{compute_types, storage_types, validation_types};
            use std::fmt::Debug;

            pub struct ProcessorA<T>(pub T);
            pub struct ProcessorB<T>(pub T);
            pub struct ProcessorC<T>(pub T);
            pub struct ValidatorX<T>(pub T);
            pub struct ValidatorY<T>(pub T);

            impl<T> Compute<T> for ProcessorA<T>
            where
                T: Clone + Debug,
                ProcessorB<T>: Process,
                ProcessorB<T>: Validate<T>,
                ValidatorX<T>: Transform<T, String>,
                compute_types::FastProcessor<T>: super::Compute<T>,
                validation_types::EmailValidator: super::Validate<String>,
            {
                type Output = T;
                fn compute(&self, input: T) -> Self::Output {
                    input
                }
            }

            impl<T> Process for ProcessorA<T>
            where
                T: Debug + Send + Sync,
                ProcessorC<T>: Compute<T>,
                ValidatorY<T>: Validate<T>,
                storage_types::MemoryCache<String, T>: super::Cache<String, T>,
                compute_types::SlowProcessor<T>: super::Process,
            {
                fn process(&self) {}
            }

            impl<T> Process for ProcessorB<T>
            where
                T: Clone + PartialEq,
                ProcessorA<T>: Compute<T>,
                ValidatorX<T>: Process,
                storage_types::DiskCache<T, String>: super::Cache<T, String>,
                validation_types::NumberValidator<T>: super::Validate<T>,
            {
                fn process(&self) {}
            }

            impl<T> Validate<T> for ProcessorB<T>
            where
                T: Debug + Clone + Default,
                ProcessorC<T>: Transform<T, Vec<T>>,
                compute_types::FastProcessor<T>: super::Process,
                storage_types::MemoryCache<T, Vec<u8>>: super::Serialize<(T, Vec<u8>)>,
            {
                fn validate(&self, _data: T) -> bool {
                    true
                }
            }

            impl<T> Compute<T> for ProcessorC<T>
            where
                T: Clone + Send + 'static,
                ProcessorA<T>: Process,
                ValidatorY<T>: Validate<T>,
                ValidatorX<T>: Transform<T, String>,
                validation_types::DataTransformer<T>: super::Transform<T, String>,
                compute_types::SlowProcessor<T>: super::Compute<T>,
            {
                type Output = T;
                fn compute(&self, input: T) -> Self::Output {
                    input
                }
            }

            impl<T> Transform<T, String> for ValidatorX<T>
            where
                T: Debug + Clone,
                ProcessorA<T>: Compute<T>,
                ProcessorB<T>: Process,
                ValidatorY<T>: Validate<T>,
            {
                fn transform(&self, _from: T) -> String {
                    String::new()
                }
            }

            impl<T> Process for ValidatorX<T>
            where
                T: Send + Sync + Debug,
                ProcessorC<T>: Compute<T>,
            {
                fn process(&self) {}
            }

            impl<T> Validate<T> for ValidatorY<T>
            where
                T: Clone + Debug + Default + PartialEq,
                ProcessorA<T>: Process,
                ProcessorB<T>: Validate<T>,
                ValidatorX<T>: Transform<T, String>,
            {
                fn validate(&self, _data: T) -> bool {
                    true
                }
            }

            impl<T> Transform<T, Vec<T>> for ProcessorC<T>
            where
                T: Clone + Debug + Send,
                ProcessorA<T>: Compute<T>,
                ProcessorB<T>: Validate<T>,
                ValidatorX<T>: Process,
                ValidatorY<T>: Validate<T>,
            {
                fn transform(&self, from: T) -> Vec<T> {
                    vec![from]
                }
            }
        }
    };
}

dense_cycle_module!(decycle, complex_default);
dense_cycle_module!(decycle(support_infinite_cycle = false), complex_bounded);

macro_rules! dense_cycle_test_body {
    ($m:ident) => {{
        use crate::{Compute, Process, Transform, Validate};

        let processor_a = $m::ProcessorA(42i32);
        let processor_b = $m::ProcessorB("test".to_string());
        let validator_x = $m::ValidatorX(3.5f64);
        let validator_y = $m::ValidatorY(true);

        processor_a.process();
        let result = processor_a.compute(100);
        assert_eq!(result, 100);

        processor_b.process();
        assert!(processor_b.validate("test".to_string()));

        validator_x.process();
        let string_result = validator_x.transform(2.5f64);
        assert_eq!(string_result, String::new());

        assert!(validator_y.validate(false));

        // test_generic_type_coinduction
        let int_processor = $m::ProcessorA(123);
        let string_processor = $m::ProcessorB("hello".to_string());
        let float_validator = $m::ValidatorX(3.5);
        int_processor.process();
        assert_eq!(int_processor.compute(456), 456);
        string_processor.process();
        assert!(string_processor.validate("world".to_string()));
        float_validator.process();
        assert_eq!(float_validator.transform(2.5), String::new());

        // test_multi_trait_implementations
        let processor_c = $m::ProcessorC(42);
        let validator_y2 = $m::ValidatorY("test".to_string());
        assert_eq!(processor_c.compute(100), 100);
        assert_eq!(processor_c.transform(50), vec![50]);
        assert!(validator_y2.validate("validation".to_string()));
    }};
}

#[test]
fn dense_cycle_default_mode() {
    dense_cycle_test_body!(complex_default);
}

#[test]
fn dense_cycle_bounded_mode() {
    dense_cycle_test_body!(complex_bounded);
}
