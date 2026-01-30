#[decycle::decycle]
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
