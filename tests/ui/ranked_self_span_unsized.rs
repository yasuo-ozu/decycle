//! A diagnostic on a `Self` the ranked engine copies from the user's signature points at that
//! `Self`. An impl for an unsized type taking `other: Self` is `E0277` on the user's impl and on
//! each generated copy of the method. The floor's copy used to respell `Self` (at the macro's call
//! site), so one of the errors pointed at `#[decycle]`, with the suggestion `&#[decycle]`; it now
//! points at the user's `Self` and merges with the error on the user's impl.
use decycle::decycle;

#[decycle]
mod m {
    #[decycle]
    pub trait Tr {
        fn take(&self, other: Self) -> u32;
    }

    pub struct B;

    impl Tr for str
    where
        B: Tr,
    {
        fn take(&self, other: Self) -> u32 {
            other.len() as u32
        }
    }

    impl Tr for B
    where
        str: Tr,
    {
        fn take(&self, _other: Self) -> u32 {
            0
        }
    }
}

fn main() {}
