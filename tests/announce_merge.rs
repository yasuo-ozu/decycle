//! announce 0.2 merges several announcements of one item into one trampoline. `#[decycle]` on a
//! trait outside a module expands to an `#[announce]`, so it merges with the announcements written
//! beside it (in 0.1 each one made its own trampoline, and the item failed with E0252).

mod merged {
    // `#[decycle]` first: it expands to `#[announce(.., allow_std_prelude = true)]`, which meets the
    // `#[announce]` below. `Option` and `String` are accepted because the options are OR-ed.
    #[decycle::decycle]
    #[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
    pub trait CountA {
        fn count_a(&self) -> usize;
        fn tagged_a(&self, tag: String) -> Option<usize>;
    }

    // `#[announce]` first: it sees `#[decycle]` as an attribute that expands, and the announcement
    // that `#[decycle]` makes is merged into it.
    #[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
    #[decycle::decycle]
    pub trait CountB {
        fn count_b(&self) -> usize;
        fn tagged_b(&self, tag: String) -> Option<usize>;
    }
}

#[decycle::decycle]
mod uses_a {
    #[decycle]
    pub use crate::merged::CountA;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl CountA for A
    where
        B: CountA,
    {
        fn count_a(&self) -> usize {
            self.0.as_ref().map_or(0, |b| b.count_a() + 1)
        }
        fn tagged_a(&self, tag: String) -> Option<usize> {
            self.0.as_ref().map(|b| b.count_a() + tag.len())
        }
    }
    impl CountA for B
    where
        A: CountA,
    {
        fn count_a(&self) -> usize {
            self.0.as_ref().map_or(0, |a| a.count_a() + 1)
        }
        fn tagged_a(&self, tag: String) -> Option<usize> {
            self.0.as_ref().map(|a| a.count_a() + tag.len())
        }
    }
}

#[decycle::decycle]
mod uses_b {
    #[decycle]
    pub use crate::merged::CountB;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl CountB for A
    where
        B: CountB,
    {
        fn count_b(&self) -> usize {
            self.0.as_ref().map_or(0, |b| b.count_b() + 10)
        }
        fn tagged_b(&self, tag: String) -> Option<usize> {
            self.0.as_ref().map(|b| b.count_b() + tag.len())
        }
    }
    impl CountB for B
    where
        A: CountB,
    {
        fn count_b(&self) -> usize {
            self.0.as_ref().map_or(0, |a| a.count_b() + 10)
        }
        fn tagged_b(&self, tag: String) -> Option<usize> {
            self.0.as_ref().map(|a| a.count_b() + tag.len())
        }
    }
}

#[test]
fn decycle_then_announce_make_one_trampoline() {
    use merged::CountA as _;
    use uses_a::{A, B};
    let a = A(Some(Box::new(B(Some(Box::new(A(None)))))));
    assert_eq!(a.count_a(), 2);
    assert_eq!(a.tagged_a("xy".into()), Some(3));
}

#[test]
fn announce_then_decycle_make_one_trampoline() {
    use merged::CountB as _;
    use uses_b::{A, B};
    let a = A(Some(Box::new(B(Some(Box::new(A(None)))))));
    assert_eq!(a.count_b(), 20);
    assert_eq!(a.tagged_b("xy".into()), Some(12));
}

// An announced item inside a `#[decycle]` module (a struct beside a derive, and a trait that is not
// `#[decycle]`) passes through the module as written: its own `#[announce]` expands after the
// module, and the module's impls still resolve their cycle.
#[decycle::decycle]
mod announced_inside {
    #[decycle]
    pub use crate::merged::CountA;

    #[derive(Clone, Debug, PartialEq)]
    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub struct Plain(pub u8);

    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub trait Local {
        fn local(&self) -> u8;
    }
    impl Local for Plain {
        fn local(&self) -> u8 {
            self.0
        }
    }

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl CountA for A
    where
        B: CountA,
    {
        fn count_a(&self) -> usize {
            self.0.as_ref().map_or(0, |b| b.count_a() + 1)
        }
        fn tagged_a(&self, tag: String) -> Option<usize> {
            self.0.as_ref().map(|b| b.count_a() + tag.len())
        }
    }
    impl CountA for B
    where
        A: CountA,
    {
        fn count_a(&self) -> usize {
            self.0.as_ref().map_or(0, |a| a.count_a() + 1)
        }
        fn tagged_a(&self, tag: String) -> Option<usize> {
            self.0.as_ref().map(|a| a.count_a() + tag.len())
        }
    }
}

#[test]
fn announced_items_inside_a_decycle_module() {
    use announced_inside::{Local as _, Plain, A, B};
    use merged::CountA as _;
    let a = A(Some(Box::new(B(None))));
    assert_eq!(a.count_a(), 1);
    assert_eq!(Plain(7).clone().local(), 7);
}
