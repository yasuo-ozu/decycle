//! `alter_macro_name` was an option of the trait-side `#[decycle]`, which was removed in 0.6; a trait
//! outside a `#[decycle] mod` is announced with `#[announce]`, and `#[decycle] use`d by its own name.
#[decycle::decycle(alter_macro_name = Renamed)]
pub trait T {
    fn f(&self);
}

fn main() {}
