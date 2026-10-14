//! `#[decycle] use` of an announced item that is not a trait is rejected at the `use`, in the user's
//! crate, not at the definition, which may be in another one.
pub mod d {
    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub struct NotTrait(pub u8);
}

#[decycle::decycle]
mod m {
    #[decycle]
    use crate::d::NotTrait;

    pub struct A;
}

fn main() {}
