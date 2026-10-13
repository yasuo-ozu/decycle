//! `decycle::check` rejects return-position `impl Trait` in a trait method at its definition. (The
//! trait path is absolute: announce checks the paths of the trait first, and reports a bare one.)
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
pub trait Ca {
    fn ca(&self, n: usize) -> impl ::core::iter::Iterator<Item = u8>;
}

fn main() {}
