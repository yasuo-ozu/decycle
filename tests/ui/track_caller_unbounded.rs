//! `#[track_caller]` on a method in an UNBOUNDED ranked cycle is rejected up-front: past the
//! recursion floor the re-entry fn-pointer transmute can't carry the caller location, so
//! `Location::caller()` would be silently wrong. (Bounded mode and `#[decycle(structural)]` are fine.)
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Ev {
        fn ev(&self);
    }
    pub struct L;
    pub struct R;
    impl Ev for L
    where
        R: Ev,
    {
        #[track_caller]
        fn ev(&self) {}
    }
    impl Ev for R
    where
        L: Ev,
    {
        fn ev(&self) {}
    }
}

fn main() {}
