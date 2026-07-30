//! `async fn` in a `#[decycle]` (ranked) cycle is rejected up-front — its opaque `impl Future` return
//! can't be threaded through the rank rewrite (the raw symptom was a confusing `E0308`). Fail closed
//! with an actionable message pointing at the boxed-future workaround.
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Ev {
        async fn ev(&self, depth: u64) -> u64;
    }
    pub struct A;
    pub struct B;
    impl Ev for A
    where
        B: Ev,
    {
        async fn ev(&self, depth: u64) -> u64 {
            if depth == 0 {
                0
            } else {
                B.ev(depth - 1).await
            }
        }
    }
    impl Ev for B
    where
        A: Ev,
    {
        async fn ev(&self, depth: u64) -> u64 {
            A.ev(depth).await
        }
    }
}

fn main() {}
