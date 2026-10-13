//! `async fn` in an ANNOUNCED trait that is not checked with `decycle::check` is still rejected where
//! the `#[decycle]` module receives it: the trait is not defined in the module, so the module-side
//! rejection of the trait's own `async fn` (`async_fn_ranked.rs`) does not apply, and the impl's
//! `async fn` is what is reported. (Add `decycle::check` to the trait to get the error at its
//! definition: `check_async_fn.rs`.)
#[decycle::announce::announce(crate = ::decycle::announce)]
pub trait Ev {
    async fn ev(&self, depth: u64) -> u64;
}

#[decycle::decycle]
mod m {
    #[decycle]
    use crate::Ev;

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
