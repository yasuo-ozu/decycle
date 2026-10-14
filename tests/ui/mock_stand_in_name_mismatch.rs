//! A stand-in named differently from its real trait, `#[decycle] use`d without an `as` rename: the
//! module's `impl Size for ..` can only be matched under the real trait's name, so the stand-in is
//! rejected by name. (It used to fail with an unrelated E0275, the cycle left unexpanded.)
pub mod real {
    pub trait Size {
        fn size(&self) -> usize;
    }
}

pub mod stand {
    #[decycle::announce::announce(crate = ::decycle::announce, mock = crate::real::Size)]
    pub trait Sz {
        fn size(&self) -> usize;
    }
}

#[decycle::decycle]
mod tree {
    use crate::real::Size;
    #[decycle]
    use crate::stand::Sz;

    pub struct Node(pub Vec<Leaf>);
    pub struct Leaf(pub Option<Box<Node>>);

    impl Size for Node
    where
        Leaf: Size,
    {
        fn size(&self) -> usize {
            1 + self.0.iter().map(|l| l.size()).sum::<usize>()
        }
    }

    impl Size for Leaf
    where
        Node: Size,
    {
        fn size(&self) -> usize {
            self.0.as_ref().map_or(1, |n| n.size())
        }
    }
}

fn main() {}
