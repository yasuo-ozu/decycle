//! When the `#[decycle]` trait is declared in the module, an impl that spells the concrete self type
//! where the declaration says `Self` is silently canonicalized (see
//! `tests/structural_fix_regressions.rs`). When the trait is `use`-imported instead, there is no
//! declaration to compare against — decycle cannot tell a `Self` position written concretely from a
//! parameter the trait genuinely declares as `&A` — so it says so, instead of letting the mismatch
//! surface as an `E0053` whose "help" tells the user to write `&__ATerm_<nonce>`.
use decycle::decycle;

mod outer {
    pub trait Vis {
        fn merge(&self, other: &Self) -> i64;
    }
}

#[decycle(structural)]
mod m {
    #[decycle]
    use super::outer::Vis;

    pub struct A(pub i64);
    impl Vis for A
    where
        A: Vis,
    {
        fn merge(&self, other: &A) -> i64 {
            self.0 + other.0
        }
    }
}

fn main() {}
