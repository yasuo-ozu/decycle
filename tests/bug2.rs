use decycle::decycle;

#[allow(dead_code)]
#[decycle]
pub trait MyTrait<'a> {
    fn f<'b>(&'a self, _: &'b [u8]) -> usize {
        0
    }
}
#[decycle]
mod m {
    #[decycle]
    use super::MyTrait;
    struct MyStruct<'a, 'b, const N: usize, T> {
        _marker: ::core::marker::PhantomData<(&'a T, &'b [(); N])>,
    }
    #[automatically_derived]
    impl<'a, 'b, const N: usize, T> MyTrait<'a> for MyStruct<'a, 'b, N, T> {}
}
