//! A trait announced with `macro_name = WalkMac` (the 0.5 `alter_macro_name`) is `#[decycle] use`d
//! through its trampoline `WalkMac`. The module's `impl Walk for ..` can only be matched to the trait
//! by name, so `use` the trampoline `as Walk` (and the trait itself with a plain `use`). (Left
//! unrenamed, it used to fail with an unrelated E0275, the cycle unexpanded.)
pub mod d {
    #[decycle::announce::announce(crate = ::decycle::announce, macro_name = WalkMac)]
    pub trait Walk {
        fn walk(&self, depth: usize) -> usize;
    }
}

#[decycle::decycle]
mod m {
    use crate::d::Walk;
    #[decycle]
    use crate::d::WalkMac;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Walk for A
    where
        B: Walk,
    {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |b| b.walk(depth + 1))
        }
    }

    impl Walk for B
    where
        A: Walk,
    {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |a| a.walk(depth + 1))
        }
    }
}

fn main() {}
