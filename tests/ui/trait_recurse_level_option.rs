//! `recurse_level` is an option of the `#[decycle]` module; on a trait it used to be dropped
//! silently. It is now rejected, pointing at where it goes.
#[decycle::decycle(recurse_level = 3)]
pub trait T {
    fn f(&self) -> usize;
}

fn main() {}
