//! announce-std's `Display` is the real trait in the type namespace as well as the trampoline, so
//! `#[decycle] use announce_std::fmt::Display;` is all a module writes. Keeping the real trait in
//! scope under the same name beside it defines `Display` twice (`tests/mock_stand_in.rs` has the
//! spelling that works).
#[decycle::decycle]
mod m {
    use core::fmt::Display;
    #[decycle]
    use announce_std::fmt::Display;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Display for A
    where
        B: Display,
    {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            match &self.0 {
                Some(b) => write!(f, "A({})", b),
                None => write!(f, "A"),
            }
        }
    }

    impl Display for B
    where
        A: Display,
    {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            match &self.0 {
                Some(a) => write!(f, "B({})", a),
                None => write!(f, "B"),
            }
        }
    }
}

fn main() {}
