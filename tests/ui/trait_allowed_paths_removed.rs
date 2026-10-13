//! `allowed_paths = [..]` (the type-leak allowlist) was removed in 0.6 with the trait-side
//! `#[decycle]`. Nothing replaces it: write the paths absolutely, or `allow_std_prelude = true` on
//! `#[announce]` for the bare std prelude names.
#[decycle::decycle(allowed_paths = [::core::primitive::u32])]
pub trait T {
    fn f(&self) -> u32;
}

fn main() {}
