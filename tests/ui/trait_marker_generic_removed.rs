//! The value of a removed option can contain a comma: `marker = M<u8, u16>`. Skipping it up to the
//! first top-level comma left `u16>` behind and replaced the "removed in 0.6" help with a generic
//! "keyword arguments should be one of" error.
pub struct M<A, B>(pub A, pub B);

#[decycle::decycle(marker = M<u8, u16>)]
pub trait T {
    fn f(&self) -> M<u8, u16>;
}

fn main() {}
