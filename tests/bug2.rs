use decycle::decycle;
use m::MyStruct;

#[allow(dead_code)]
#[decycle]
pub trait MyTrait<'a> {
    type MyTrait;
    type T;
    fn f<'b>(&'a self, _: &'b [u8]) -> usize {
        0
    }
}

impl MyTrait<'static> for () {
    type MyTrait = ();
    type T = ();
}

#[decycle]
mod m {
    #[decycle]
    use super::MyTrait;

    #[derive(Default)]
    pub struct MyStruct<'a, 'b, const N: usize, T> {
        _marker: ::core::marker::PhantomData<(&'a T, &'b [(); N])>,
    }

    impl<'a, 'b, const N: usize, T> MyTrait<'a> for MyStruct<'a, 'b, N, T>
    // where
    //     (): MyTrait<'b, MyTrait = T, T = T>,
    {
        type MyTrait = T;
        type T = T;
        fn f<'c>(&'a self, i: &'c [u8]) -> usize {
            // <() as MyTrait<'b>>::f(&(), i)
            0
        }
    }
}

#[test]
fn run_f() {
    let s: MyStruct<'static, 'static, 123, ()> = Default::default();
    assert_eq!(s.f(&[]), 0);
}
