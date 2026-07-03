#[decycle::decycle]
pub trait Parse<Item>: ::core::marker::Sized {
    fn parse<I: ::core::iter::Iterator<Item = Item>>(stream: I);
}

#[decycle::decycle]
mod m {
    #[decycle]
    use super::Parse;

    // Compile-only regression test: `S` only needs to exist as an impl target, never
    // constructed.
    #[allow(dead_code)]
    struct S;

    impl<Item> Parse<Item> for S {
        fn parse<I: ::core::iter::Iterator<Item = Item>>(_: I) {
            todo!()
        }
    }
}
