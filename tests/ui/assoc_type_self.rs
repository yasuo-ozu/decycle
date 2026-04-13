#[decycle::decycle]
mod m {
    #[decycle]
    trait Foo {
        type Assoc: Foo;
    }

    #[decycle]
    trait Bar {}

    struct S;

    impl Foo for S
    where
        S: Bar,
    {
        type Assoc = Self;
    }

    impl Bar for S where S: Foo {}
}

fn main() {}
