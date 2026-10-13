//! `#[decycle]` on a trait outside a `#[decycle] mod` was removed in 0.6: the trait is announced
//! with `#[announce]` instead. The error says so, and the trait itself is still emitted (no
//! cascade).
#[decycle::decycle]
pub trait T {
    fn f(&self);
}

fn main() {}
