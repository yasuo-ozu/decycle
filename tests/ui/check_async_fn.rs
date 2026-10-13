//! `decycle::check` rejects an `async fn` in a trait method at its definition, like a
//! return-position `impl Trait`: its `impl Future` return is an opaque type neither engine can carry
//! through a cycle. Without the check the trait announces fine, and the error only appears later,
//! where a `#[decycle]` module uses it.
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
pub trait Fut {
    async fn go(&self) -> usize;
}

fn main() {}
