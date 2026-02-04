use decycle::decycle;

#[allow(dead_code)]
#[decycle]
pub trait MyTrait {}
#[decycle]
mod m {
    #[decycle]
    use super::MyTrait;
    struct MyStruct<'lifetime, S> {
        _marker: ::core::marker::PhantomData<(&'lifetime (), S)>,
    }
    #[automatically_derived]
    impl<'lifetime, S> MyTrait for MyStruct<'lifetime, S> {}
}
