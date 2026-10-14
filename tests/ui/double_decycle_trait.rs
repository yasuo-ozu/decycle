//! `#[decycle]` twice on one trait used to announce it twice, and fail later on the redefinition of
//! the trampoline macro. The outer invocation now detects the still-pending inner attribute and
//! says what actually happened.
#[decycle::decycle]
#[decycle::decycle]
pub trait T {
    fn f(&self) -> usize;
}

fn main() {}
