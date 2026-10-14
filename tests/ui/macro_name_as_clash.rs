//! `#[decycle] use d::WalkMac as Walk;` beside `use d::Walk;` when a macro named `Walk` shares the
//! path of the trait: `use d::Walk;` imports the trait AND that macro, and a `use` imports every
//! namespace of what it names, so `Walk` is defined twice in the macro namespace. Use the trampoline
//! under its own name (`#[decycle] use d::WalkMac;`, see `tests/macro_name_clash.rs`).
pub mod d {
    #[decycle::announce::announce(crate = ::decycle::announce, macro_name = WalkMac)]
    pub trait Walk {
        fn walk(&self, depth: usize) -> usize;
    }

    macro_rules! walk_m {
        ($($t:tt)*) => {};
    }
    pub(crate) use walk_m as Walk;
}

#[decycle::decycle]
mod m {
    use crate::d::Walk;
    #[decycle]
    use crate::d::WalkMac as Walk;

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
