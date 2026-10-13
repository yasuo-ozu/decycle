//! `decycle::check` takes no argument, or `structural`.
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check(ranked))]
pub trait Tr {
    fn tr(&self) -> usize;
}

fn main() {}
