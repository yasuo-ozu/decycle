//! A trait announced with `macro_name = WalkMac` whose name is ALSO taken in the macro namespace
//! at the same path (a derive or attribute macro `Walk`, here a `macro_rules!` re-exported under
//! that name). `use path::Walk;` then imports the trait AND that macro, so a second import of
//! something named `Walk` in the macro namespace (`#[decycle] use path::WalkMac as Walk;`) is
//! E0252: a `use` cannot import only one namespace. The trampoline is therefore `use`d under its
//! own name, and the trait is named by the plain `use`.
#![allow(dead_code)]

#[allow(unused_macros)]
pub mod d {
    #[decycle::announce::announce(crate = ::decycle::announce, macro_name = WalkMac)]
    pub trait Walk {
        fn walk(&self, depth: usize) -> usize;
    }

    pub trait Unrelated {}

    macro_rules! walk_m {
        ($($t:tt)*) => {};
    }
    // The macro that keeps the trait's name in the macro namespace.
    pub(crate) use walk_m as Walk;
}

#[decycle::decycle]
mod ranked {
    use crate::d::Walk;
    #[decycle]
    use crate::d::WalkMac;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Walk for A
    where
        B: Walk,
    {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |b| b.walk(depth + 1))
        }
    }

    impl Walk for B
    where
        A: Walk,
    {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |a| a.walk(depth + 1) + 10)
        }
    }
}

#[test]
fn ranked_unrenamed_trampoline_next_to_a_same_named_macro() {
    use crate::d::Walk as _;
    let a = ranked::A(Some(Box::new(ranked::B(Some(Box::new(ranked::A(None)))))));
    // inner A at depth 2: 2; B: 2 + 10 = 12; outer A: 12
    assert_eq!(a.walk(0), 12);
}

#[decycle::decycle(structural)]
mod structural {
    use crate::d::Walk;
    #[decycle]
    use crate::d::WalkMac;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Walk for A
    where
        B: Walk,
    {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |b| b.walk(depth + 1))
        }
    }

    impl Walk for B
    where
        A: Walk,
    {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |a| a.walk(depth + 1) + 10)
        }
    }
}

#[test]
fn structural_unrenamed_trampoline_next_to_a_same_named_macro() {
    use crate::d::Walk as _;
    let a = structural::A(Some(Box::new(structural::B(Some(Box::new(
        structural::A(None),
    ))))));
    assert_eq!(a.walk(0), 12);
}

// The trampoline beside a trait of the module and another `#[decycle] use` (renamed), with the trait
// `use`d in a group.
pub mod e {
    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub trait Plain {
        fn plain(&self) -> usize;
    }
}

macro_rules! mixed {
    ($attr:meta, $name:ident) => {
        #[$attr]
        mod $name {
            #[decycle]
            use crate::d::WalkMac;
            #[allow(unused_imports)]
            use crate::d::{Unrelated as _, Walk};
            #[decycle]
            use crate::e::Plain as P;

            #[decycle]
            trait Local {
                fn local(&self) -> usize;
            }

            pub struct A(pub Option<Box<B>>);
            pub struct B(pub Option<Box<C>>);
            pub struct C(pub Option<Box<A>>);

            impl Walk for A
            where
                B: P,
            {
                fn walk(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |b| b.plain() + depth)
                }
            }

            impl P for B
            where
                C: Local,
            {
                fn plain(&self) -> usize {
                    self.0.as_ref().map_or(1, |c| c.local() + 10)
                }
            }

            impl Local for C
            where
                A: Walk,
            {
                fn local(&self) -> usize {
                    self.0.as_ref().map_or(100, |a| a.walk(1) + 1000)
                }
            }
        }
    };
}

mixed!(decycle::decycle, ranked_mixed);
mixed!(decycle::decycle(structural), structural_mixed);

#[test]
fn ranked_unrenamed_trampoline_with_other_routed_traits() {
    use crate::d::Walk as _;
    let a = ranked_mixed::A(Some(Box::new(ranked_mixed::B(Some(Box::new(
        ranked_mixed::C(Some(Box::new(ranked_mixed::A(None)))),
    ))))));
    // A(None).walk(1) = 1; C.local = 1 + 1000; B.plain = 1001 + 10; A.walk(0) = 1011 + 0
    assert_eq!(a.walk(0), 1011);
}

#[test]
fn structural_unrenamed_trampoline_with_other_routed_traits() {
    use crate::d::Walk as _;
    let a = structural_mixed::A(Some(Box::new(structural_mixed::B(Some(Box::new(
        structural_mixed::C(Some(Box::new(structural_mixed::A(None)))),
    ))))));
    assert_eq!(a.walk(0), 1011);
}

// The rest of the module goes through the trampoline whole: a `macro_rules!` definition before its
// uses, a nested module, a `#[cfg]`, doc comments and other attributes on the module and its items.
macro_rules! everything {
    ($attr:meta, $name:ident) => {
        /// A documented module.
        #[$attr]
        #[allow(clippy::all)]
        pub(crate) mod $name {
            use crate::d::Walk;
            #[decycle]
            use crate::d::WalkMac;

            macro_rules! bump {
                ($e:expr) => {
                    $e + 1
                };
            }

            pub mod inner {
                pub const BASE: usize = 100;
            }

            /// A documented type.
            #[derive(Debug, Clone)]
            pub struct A(pub Option<Box<B>>);
            #[derive(Debug, Clone)]
            pub struct B(pub Option<Box<A>>);

            #[cfg(any())]
            pub struct Gone;

            impl Walk for A
            where
                B: Walk,
            {
                fn walk(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |b| bump!(b.walk(depth + 1)))
                }
            }

            impl Walk for B
            where
                A: Walk,
            {
                fn walk(&self, depth: usize) -> usize {
                    self.0
                        .as_ref()
                        .map_or(inner::BASE + depth, |a| bump!(a.walk(depth + 1)))
                }
            }

            impl A {
                pub fn depth_hint(&self) -> usize {
                    inner::BASE
                }
            }
        }
    };
}

everything!(decycle::decycle, ranked_everything);
everything!(decycle::decycle(structural), structural_everything);

#[test]
fn ranked_rest_of_the_module_goes_through_the_trampoline() {
    use crate::d::Walk as _;
    let a = ranked_everything::A(Some(Box::new(ranked_everything::B(None))));
    // B(None) at depth 1: 100 + 1 = 101; A: 101 + 1
    assert_eq!(a.walk(0), 102);
    assert_eq!(a.depth_hint(), 100);
}

#[test]
fn structural_rest_of_the_module_goes_through_the_trampoline() {
    use crate::d::Walk as _;
    let a = structural_everything::A(Some(Box::new(structural_everything::B(None))));
    assert_eq!(a.walk(0), 102);
    assert_eq!(a.depth_hint(), 100);
}

// A trampoline that is reached under another name only because it is re-exported under an alias
// (`pub use d::Plain2 as Step;`) is `use`d the same way: the trait by a plain `use`, the trampoline
// by its alias.
pub mod alias_src {
    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub trait Plain2 {
        fn plain2(&self, depth: usize) -> usize;
    }
}

pub mod alias {
    pub use crate::alias_src::Plain2 as Step;
}

macro_rules! reexported {
    ($attr:meta, $name:ident) => {
        #[$attr]
        mod $name {
            #[decycle]
            use crate::alias::Step;
            use crate::alias_src::Plain2;

            pub struct A(pub Option<Box<B>>);
            pub struct B(pub Option<Box<A>>);

            impl Plain2 for A
            where
                B: Plain2,
            {
                fn plain2(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |b| b.plain2(depth + 1))
                }
            }

            impl Plain2 for B
            where
                A: Plain2,
            {
                fn plain2(&self, depth: usize) -> usize {
                    self.0.as_ref().map_or(depth, |a| a.plain2(depth + 1) + 10)
                }
            }
        }
    };
}

reexported!(decycle::decycle, ranked_reexported);
reexported!(decycle::decycle(structural), structural_reexported);

#[test]
fn ranked_trampoline_reexported_under_an_alias() {
    use crate::alias_src::Plain2 as _;
    let a = ranked_reexported::A(Some(Box::new(ranked_reexported::B(Some(Box::new(
        ranked_reexported::A(None),
    ))))));
    assert_eq!(a.plain2(0), 12);
}

#[test]
fn structural_trampoline_reexported_under_an_alias() {
    use crate::alias_src::Plain2 as _;
    let a = structural_reexported::A(Some(Box::new(structural_reexported::B(Some(Box::new(
        structural_reexported::A(None),
    ))))));
    assert_eq!(a.plain2(0), 12);
}

// No cycle at all: the structural engine passes the module through, the ranked one expands it all
// the same. Either way the module keeps its items.
macro_rules! acyclic {
    ($attr:meta, $name:ident) => {
        #[$attr]
        mod $name {
            use crate::d::Walk;
            #[decycle]
            use crate::d::WalkMac;

            pub struct Leaf(pub usize);

            impl Walk for Leaf {
                fn walk(&self, depth: usize) -> usize {
                    self.0 + depth
                }
            }
        }
    };
}

acyclic!(decycle::decycle, ranked_acyclic);
acyclic!(decycle::decycle(structural), structural_acyclic);

#[test]
fn acyclic_modules_keep_their_items() {
    use crate::d::Walk as _;
    assert_eq!(ranked_acyclic::Leaf(5).walk(1), 6);
    assert_eq!(structural_acyclic::Leaf(5).walk(1), 6);
}
