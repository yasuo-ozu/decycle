//! `async fn` in a `#[decycle(structural)]` cycle is rejected up-front. Its returned `impl Future` is
//! an opaque type the layout cast can't reinterpret — the dispatch would `transmute_copy` an un-polled
//! future (silent UB when its size happens to equal the return type's). Fail closed with a clear error.
use decycle::decycle;

#[decycle(structural)]
mod m {
    #[decycle]
    pub trait Ev {
        async fn ev(&self, depth: u64) -> u64;
    }
    pub struct A;
    impl Ev for A
    where
        A: Ev,
    {
        async fn ev(&self, depth: u64) -> u64 {
            if depth == 0 {
                0
            } else {
                self.ev(depth - 1).await
            }
        }
    }
}

fn main() {}
