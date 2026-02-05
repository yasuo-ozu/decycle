#[allow(unused)]
#[decycle::decycle]
trait Unparse {
    fn unparse(&self, _: usize);
}

#[decycle::decycle]
mod m {
    #[allow(unused)]
    struct S;
    impl Unparse for S {
        fn unparse(&self, i: usize) {
            if i == 0 {
                return;
            }
            <_ as Unparse>::unparse(self, i - 1);
        }
    }
    #[decycle]
    use super::Unparse;
}
