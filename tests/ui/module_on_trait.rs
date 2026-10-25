//! `module` is the path of a `#[decycle]` module; on a trait (which `#[decycle]` announces) it would
//! be dropped silently, so it is rejected, pointing at where it goes.
#[decycle::decycle(module = crate::T)]
pub trait T {
    fn f(&self) -> usize;
}

fn main() {}
