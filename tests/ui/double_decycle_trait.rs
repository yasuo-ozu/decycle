//! `#[decycle]` twice on one trait used to expand twice and die with E0252 ("the name … is
//! defined multiple times", plus a rustc suggestion that is not valid Rust). The outer
//! invocation now detects the still-pending inner attribute and says what actually happened.
#[decycle::decycle]
#[decycle::decycle]
trait Twice {
    fn value(&self) -> i32;
}

fn main() {}
