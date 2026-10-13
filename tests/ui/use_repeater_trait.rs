//! A trait announced with a `repeater` carries its relative types through a table that decycle does
//! not read, so `#[decycle] use` of it is rejected rather than left to fail on an unresolved name.
mod defs {
    pub struct Local(pub usize);

    ::decycle::announce::define_repeater!(pub Repeater);

    #[decycle::announce::announce(crate = ::decycle::announce, repeater = crate::defs::Repeater)]
    pub trait Produce {
        fn produce(&self) -> Local;
    }
}

#[decycle::decycle]
mod m {
    #[decycle]
    use crate::defs::Produce;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub usize);

    impl Produce for A
    where
        B: Produce,
    {
        fn produce(&self) -> crate::defs::Local {
            crate::defs::Local(0)
        }
    }

    impl Produce for B {
        fn produce(&self) -> crate::defs::Local {
            crate::defs::Local(self.0)
        }
    }
}

fn main() {}
