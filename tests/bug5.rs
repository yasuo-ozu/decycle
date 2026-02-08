#[decycle::decycle]
pub trait Parse<Item>: ::core::marker::Sized {
    fn parse<I: ::core::iter::Iterator<Item = Item>>(stream: I);
}

#[decycle::decycle]
mod m {
    #[decycle]
    use super::Parse;

    struct S;

    impl<Item> Parse<Item> for S {
        fn parse<I: ::core::iter::Iterator<Item = Item>>(_: I) {
            todo!()
        }
    }
}
