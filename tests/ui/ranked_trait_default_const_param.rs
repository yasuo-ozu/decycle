//! The same rejection for a defaulted CONST parameter (`const N: usize = 4`): the rank argument
//! is inserted into the written list either way, and `fingerprint_expr` folds the declared
//! const params against the written args positionally — an omitted one would silently drop out
//! of the registry key.
#[decycle::decycle]
mod m {
    #[decycle]
    pub trait Ca<const N: usize = 4> {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca<4> for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca<4>,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

fn main() {}
