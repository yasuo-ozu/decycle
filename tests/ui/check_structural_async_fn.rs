//! `decycle::check(structural)` skips the rules of the ranked engine only (a default on a generic
//! parameter), not the ones both engines share: `async fn` is still rejected at the definition.
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check(structural))]
pub trait Fut<T = u8> {
    async fn go(&self, t: T) -> usize;
}

fn main() {}
