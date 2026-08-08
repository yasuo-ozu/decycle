//! Regressions for the ranked engine's path/pattern helpers.
#![allow(dead_code)]

use decycle::decycle;

// The rank argument's slot is computed from the trait DECLARATION (lifetimes first), but a use
// site may elide its lifetime arguments — and in expression position it almost always does.
// Inserting at the declaration index then either panicked the macro outright
// ("Punctuated::insert: index out of range", no span, on legal code) or, with some args present,
// silently put the user's type in the rank slot.
#[decycle]
mod lifetime_trait {
    #[decycle]
    pub trait Tr<'a> {
        fn f(&self, s: &'a str) -> usize;
    }
    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl<'a> Tr<'a> for A
    where
        B: Tr<'a>,
    {
        // Bare call with the lifetime elided.
        fn f(&self, s: &'a str) -> usize {
            Tr::f(&*self.0, s)
        }
    }
    impl<'a> Tr<'a> for B
    where
        A: Tr<'a>,
    {
        // The qself spelling reaches the same helper.
        fn f(&self, s: &'a str) -> usize {
            match &self.0 {
                Some(a) => <A as Tr>::f(a, s),
                None => s.len(),
            }
        }
    }
}

#[test]
fn elided_lifetime_bare_and_qself_calls() {
    use lifetime_trait::Tr;
    assert_eq!(lifetime_trait::B(None).f("ab"), 2);
    assert_eq!(lifetime_trait::A(Box::new(lifetime_trait::B(None))).f("abc"), 3);
}

// A destructured parameter is rebound to a generated ident. Unsuffixed, it collided with a user
// parameter of that literal name (E0415, "bound more than once").
#[decycle]
mod arg_name_collision {
    #[decycle]
    pub trait Tr {
        fn g(&self, pair: (usize, usize), z: usize) -> usize;
    }
    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl Tr for A
    where
        B: Tr,
    {
        fn g(&self, (x, y): (usize, usize), __decycle_arg1_: usize) -> usize {
            x + y + __decycle_arg1_ + self.0.g((1, 2), 3)
        }
    }
    impl Tr for B
    where
        A: Tr,
    {
        fn g(&self, (x, y): (usize, usize), __decycle_arg1_: usize) -> usize {
            x + y + __decycle_arg1_
        }
    }
}

#[test]
fn user_param_named_like_the_generated_one() {
    use arg_name_collision::Tr;
    assert_eq!(arg_name_collision::B(None).g((1, 2), 3), 6);
    assert_eq!(arg_name_collision::A(Box::new(arg_name_collision::B(None))).g((1, 2), 3), 12);
}
