//! `#[cfg]` / `#[cfg_attr]` on a REQUIRED trait-method impl inside a `#[decycle]` cycle is rejected:
//! decycle can't keep the method's generated ranked copies in lock-step with the original under
//! rustc's cfg pass. (Gate the whole `impl` or type instead; a cfg'd EXTRA, non-trait method is fine.)
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Ev {
        fn ev(&self) -> i64;
    }
    pub struct L;
    pub struct R;
    impl Ev for L
    where
        R: Ev,
    {
        #[cfg(all())]
        fn ev(&self) -> i64 {
            1
        }
    }
    impl Ev for R
    where
        L: Ev,
    {
        fn ev(&self) -> i64 {
            2
        }
    }
}

fn main() {}
