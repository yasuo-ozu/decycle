//! `marker = path` (the type-leak interning anchor) was removed in 0.6 with the trait-side
//! `#[decycle]`. Nothing replaces it: `#[announce]` requires the trait's paths to resolve everywhere.
pub struct Marker;

#[decycle::decycle(marker = Marker)]
pub trait T {
    fn f(&self) -> Marker;
}

fn main() {}
