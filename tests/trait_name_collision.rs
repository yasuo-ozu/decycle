//! Two `#[decycle]` traits that are token-identical but declared in different modules.
//!
//! The temporal carrier macro is `#[macro_export]`ed, which hoists its name to the crate root, so
//! that name must be unique per invocation. Its discriminant used to be a hash of the trait's own
//! TOKENS — which cannot separate the one case it exists for, since identical traits hash
//! identically:
//!
//! ```text
//! error[E0428]: the name `__Render_temporal_<crate>_<tokens>` is defined multiple times
//! ```
//!
//! Realistic for small traits. Each invocation now gets a fresh discriminant.
#![allow(dead_code)]

mod a {
    #[decycle::decycle]
    pub trait Render {
        fn render(&self) -> usize;
    }
}

mod b {
    // Byte-for-byte identical to `a::Render`.
    #[decycle::decycle]
    pub trait Render {
        fn render(&self) -> usize;
    }
}

// A third, in a nested module, to check the discriminant is per-invocation rather than
// per-module-depth.
mod outer {
    pub mod inner {
        #[decycle::decycle]
        pub trait Render {
            fn render(&self) -> usize;
        }
    }
}

struct A;
struct B;
struct C;

impl a::Render for A {
    fn render(&self) -> usize {
        1
    }
}
impl b::Render for B {
    fn render(&self) -> usize {
        2
    }
}
impl outer::inner::Render for C {
    fn render(&self) -> usize {
        3
    }
}

#[test]
fn token_identical_traits_in_different_modules_coexist() {
    use a::Render as _;
    use b::Render as _;
    use outer::inner::Render as _;
    assert_eq!(A.render(), 1);
    assert_eq!(B.render(), 2);
    assert_eq!(C.render(), 3);
}
