//! L-M6: a `super::super::…`-rooted path written by the user inside a `#[decycle]`
//! module breaks once `finalize` re-emits the module's items nested inside
//! `shadowing_module`/`shadowing_module::ranked_traits` — rejected outright instead.
mod outer {
    pub mod inner {
        #[decycle::decycle]
        pub mod m {
            #[decycle]
            pub trait Loop {
                fn step(&self, n: u32) -> u32;
            }
            pub struct A;
            pub struct B;
            impl Loop for A
            where
                B: Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        super::super::helper() + B.step(n - 1)
                    }
                }
            }
            impl Loop for B
            where
                A: Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        A.step(n - 1)
                    }
                }
            }
        }

        pub fn helper() -> u32 {
            0
        }
    }
}

fn main() {}
