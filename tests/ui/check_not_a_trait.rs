//! `decycle::check` applies to a trait definition.
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
pub struct S {
    pub a: u8,
}

fn main() {}
