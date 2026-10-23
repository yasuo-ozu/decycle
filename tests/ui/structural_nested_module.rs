//! A `#[decycle]` module nested in a `#[decycle(structural)]` module is rejected as in a ranked one
//! (`nested_module.rs`). It used to be expanded on its own.
#[decycle::decycle(structural)]
mod outer {
    #[decycle::decycle]
    mod inner {
        #[decycle]
        trait InnerTrait {
            fn value(&self) -> i32;
        }

        struct Data(i32);

        impl InnerTrait for Data {
            fn value(&self) -> i32 {
                self.0
            }
        }
    }
}

fn main() {}
