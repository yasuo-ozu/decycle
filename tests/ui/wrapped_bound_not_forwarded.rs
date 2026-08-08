//! The structural forwarding assertion (a compile-time TYPE assertion): stripping a WRAPPED cyclic
//! bound (`Box<Stmt>: Tr`) is only sound if the container forwards the trait — `for<X: Tr> Box<X>:
//! Tr`. `Box` has no such blanket impl, so the assertion must FAIL with a legible `E0277` whose
//! primary span is the user's own `where`-bound and whose `required by a bound in ...` note reads as
//! guidance. Pinned here so that legibility (and the good span) can't silently regress.
use decycle::decycle;

#[decycle(structural)]
mod ast {
    #[decycle]
    pub trait Tr {
        fn f(&self) -> i64;
    }
    pub enum Stmt {
        Leaf(i64),
        Node(Box<Stmt>),
    }
    impl Tr for Stmt
    where
        Box<Stmt>: Tr,
    {
        fn f(&self) -> i64 {
            match self {
                Stmt::Leaf(n) => *n,
                Stmt::Node(b) => b.f(),
            }
        }
    }
}

fn main() {}
