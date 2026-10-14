//! `structural` selects an engine, and the engines belong to the `#[decycle]` module; on a trait it
//! used to be dropped silently. It is now rejected, pointing at where it goes.
#[decycle::decycle(structural)]
pub trait T {
    fn f(&self) -> usize;
}

fn main() {}
