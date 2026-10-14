//! `decycle::check` rejects a default on a generic parameter of the trait at its definition, with
//! the same error a `#[decycle]` module gives when it receives the trait.
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
pub trait Ca<T = u8> {
    fn ca(&self, t: T) -> usize;
}

fn main() {}
