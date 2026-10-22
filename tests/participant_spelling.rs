//! Both engines read a cycle participant only by the name the module gives it, bare or
//! `self::`-qualified: the trait of an impl, the trait and the types of a bound, and the self type
//! of an impl, with or without `module = path`. A `self::` spelling is the same participant as the
//! bare one; any other path names the item itself, the opt-out, which is left as written (a cycle
//! through one fails to compile: `ui/self_type_path*.rs`, `ui/peel_wrapped_path_member.rs`).
//!
//! Every scenario is instantiated under both engines, with and without `module`.

#![allow(dead_code)]

pub mod other {
    pub struct B;
    pub struct Leaf;
}

pub mod foreign {
    /// Shares its name with the cycle member `B` of the modules below.
    pub struct B<T>(pub T);
}

// ---- `self::` spellings name the same participants as the bare ones ----

macro_rules! self_spelled {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Eval {
                fn eval(&self, n: u32) -> u32;
            }

            pub struct Expr(pub Option<Box<Term>>);
            pub struct Term(pub Option<Box<Expr>>);

            // A `self::` bound type.
            impl Eval for Expr
            where
                self::Term: Eval,
            {
                fn eval(&self, n: u32) -> u32 {
                    match &self.0 {
                        Some(t) => 1 + t.eval(n),
                        None => n,
                    }
                }
            }

            // A `self::` trait, self type, bound type and bound trait.
            impl self::Eval for self::Term
            where
                self::Expr: self::Eval,
            {
                fn eval(&self, n: u32) -> u32 {
                    match &self.0 {
                        Some(e) => 1 + e.eval(n),
                        None => n,
                    }
                }
            }
        }
    };
}

self_spelled!(ranked_self []);
self_spelled!(structural_self[structural]);
self_spelled!(ranked_self_module[module = crate::ranked_self_module]);
self_spelled!(structural_self_module [structural, module = crate::structural_self_module]);

#[test]
fn self_qualified_participants() {
    macro_rules! check {
        ($m:ident) => {{
            use $m::{Eval, Expr, Term};
            let e = Expr(Some(Box::new(Term(Some(Box::new(Expr(None)))))));
            assert_eq!(e.eval(5), 7, "{}", stringify!($m));
            // Deep enough to pass the ranked engine's floor.
            let mut deep = Expr(None);
            for _ in 0..50 {
                deep = Expr(Some(Box::new(Term(Some(Box::new(deep))))));
            }
            assert_eq!(deep.eval(0), 100, "{}", stringify!($m));
        }};
    }
    check!(ranked_self);
    check!(structural_self);
    check!(ranked_self_module);
    check!(structural_self_module);
}

// ---- a self type spelled by a path names the item itself ----

macro_rules! path_self_type {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Loop {
                fn step(&self, n: u32) -> u32;
            }

            pub struct A;
            pub struct B;

            // Not the member `B` (no `E0119` against it): an ordinary impl, emitted as written.
            impl Loop for crate::other::B {
                fn step(&self, _n: u32) -> u32 {
                    1000
                }
            }

            impl Loop for A
            where
                B: Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        B.step(n - 1) + 1
                    }
                }
            }

            impl Loop for B
            where
                A: Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        A.step(n - 1) + 1
                    }
                }
            }
        }
    };
}

path_self_type!(ranked_path_self_type []);
path_self_type!(structural_path_self_type[structural]);
path_self_type!(ranked_path_self_type_module[module = crate::ranked_path_self_type_module]);
path_self_type!(structural_path_self_type_module [structural, module = crate::structural_path_self_type_module]);

#[test]
fn path_spelled_self_type_is_not_the_member() {
    macro_rules! check {
        ($m:ident) => {{
            use $m::{Loop, A};
            assert_eq!(A.step(30), 30, "{}", stringify!($m));
            assert_eq!(crate::other::B.step(0), 1000, "{}", stringify!($m));
        }};
    }
    check!(ranked_path_self_type);
    check!(structural_path_self_type);
    check!(ranked_path_self_type_module);
    check!(structural_path_self_type_module);
}

// ---- a path-spelled wrapper around a member: the wrapper is foreign, the member is an edge ----

macro_rules! foreign_wrapper {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Loop {
                fn step(&self, n: u32) -> u32;
            }

            pub struct A;
            pub struct B;

            // The foreign `B<T>` forwards the trait; its self type is a path, so it is no member.
            impl<X: Loop> Loop for crate::foreign::B<X> {
                fn step(&self, n: u32) -> u32 {
                    self.0.step(n)
                }
            }

            // `crate::foreign::B<B>` is the foreign wrapper around the member `B`, not the member.
            impl Loop for A
            where
                crate::foreign::B<B>: Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        // Through the trait's path: the ranked engine re-emits a body where the
                        // bare name of a `#[decycle]` trait reaches only its cycle members.
                        crate::$m::Loop::step(&crate::foreign::B(B), n - 1) + 1
                    }
                }
            }

            impl Loop for B
            where
                A: Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        A.step(n - 1) + 1
                    }
                }
            }
        }
    };
}

foreign_wrapper!(ranked_foreign_wrapper []);
foreign_wrapper!(structural_foreign_wrapper[structural]);

#[test]
fn foreign_wrapper_sharing_a_members_name() {
    use crate::ranked_foreign_wrapper::Loop as _;
    use crate::structural_foreign_wrapper::Loop as _;
    assert_eq!(ranked_foreign_wrapper::A.step(30), 30);
    assert_eq!(ranked_foreign_wrapper::A.step(500), 500);
    assert_eq!(structural_foreign_wrapper::A.step(30), 30);
    assert_eq!(structural_foreign_wrapper::A.step(500), 500);
}

// ---- a wrapped bound is replaced by the obligations of ALL its parts ----

// The ranked engine peels a wrapped bound on the contract that a wrapper implements the trait when
// its type arguments do: a member is an edge, and a part that contains no member stays a premise
// on the original trait. Those parts used to be dropped, so the body's call through them had no
// obligation behind it (a run-time "re-entry fn not registered" panic, or a bound that could never
// hold accepted silently).
#[decycle::decycle]
pub mod ranked_parts {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for crate::other::Leaf {
        fn step(&self, _n: u32) -> u32 {
            100
        }
    }

    impl<X: Loop> Loop for ::std::boxed::Box<X> {
        fn step(&self, n: u32) -> u32 {
            (**self).step(n)
        }
    }

    // ⇒ `B: Loop` (an edge), `crate::other::Leaf: Loop` and `Box<crate::other::Leaf>: Loop`
    // (premises, which the impls above discharge).
    impl Loop for A
    where
        (B, crate::other::Leaf): Loop,
        Vec<(B, ::std::boxed::Box<crate::other::Leaf>)>: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                // The premises are reached through the trait's path: the engine re-emits this body
                // where the bare name of a `#[decycle]` trait reaches only its cycle members.
                B.step(n - 1)
                    + crate::ranked_parts::Loop::step(&crate::other::Leaf, n)
                    + crate::ranked_parts::Loop::step(
                        &::std::boxed::Box::new(crate::other::Leaf),
                        n,
                    )
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

#[test]
fn ranked_peel_keeps_the_member_free_parts() {
    use ranked_parts::{Loop, A};
    // Every A level adds 200 (the two leaves), every B level 1.
    assert_eq!(A.step(4), 402);
    assert_eq!(A.step(60), 30 * 200 + 30);
}

// Both engines, with the wrappers implementing the trait in the module.
macro_rules! forwarded_parts {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Loop {
                fn step(&self, n: u32) -> u32;
            }

            pub struct A;
            pub struct B;

            impl<X: Loop, Y: Loop> Loop for (X, Y) {
                fn step(&self, n: u32) -> u32 {
                    self.0.step(n) + self.1.step(n)
                }
            }

            impl Loop for crate::other::Leaf {
                fn step(&self, _n: u32) -> u32 {
                    100
                }
            }

            impl Loop for A
            where
                (B, crate::other::Leaf): Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        // Each part on its own: the ranked engine re-emits this body where a
                        // call through the tuple could reach `Leaf` only by the rank chain, which
                        // it has no ranked impl on (it is no member).
                        B.step(n - 1) + crate::$m::Loop::step(&crate::other::Leaf, n - 1)
                    }
                }
            }

            impl Loop for B
            where
                A: Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        A.step(n - 1) + 1
                    }
                }
            }
        }
    };
}

forwarded_parts!(ranked_forwarded_parts []);
forwarded_parts!(structural_forwarded_parts[structural]);

#[test]
fn wrapped_bound_with_a_member_free_part() {
    use crate::ranked_forwarded_parts::Loop as _;
    use crate::structural_forwarded_parts::Loop as _;
    assert_eq!(ranked_forwarded_parts::A.step(5), 302);
    assert_eq!(structural_forwarded_parts::A.step(5), 302);
    // Past the ranked engine's floor: every impl's re-entry fn is registered, the premise
    // `crate::other::Leaf: Loop` notwithstanding.
    assert_eq!(ranked_forwarded_parts::A.step(61), 3130);
    assert_eq!(structural_forwarded_parts::A.step(61), 3130);
}

// ---- `module = path` qualifies `self::X` as it qualifies `X` ----

macro_rules! announced_self_path {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Eval {
                fn eval(&self) -> usize;
            }

            // `#[announce]` rejects the relative `self::Term`: it compiles because the module
            // spells it `crate::..::Term` before the announcement expands.
            #[decycle::announce::announce(crate = ::decycle::announce)]
            pub struct Expr(pub ::core::option::Option<::std::boxed::Box<self::Term>>);

            pub struct Term(pub Option<Box<Expr>>);

            impl Eval for Expr
            where
                Term: Eval,
            {
                fn eval(&self) -> usize {
                    self.0.as_ref().map_or(1, |t| t.eval() + 1)
                }
            }

            impl Eval for Term
            where
                Expr: Eval,
            {
                fn eval(&self) -> usize {
                    self.0.as_ref().map_or(1, |e| e.eval() + 1)
                }
            }
        }
    };
}

announced_self_path!(ranked_announced_self_path[module = crate::ranked_announced_self_path]);
announced_self_path!(structural_announced_self_path [structural, module = crate::structural_announced_self_path]);

#[test]
fn self_path_is_qualified_for_announce() {
    macro_rules! check {
        ($m:ident) => {{
            use $m::{Eval, Expr, Term};
            let e = Expr(Some(Box::new(Term(Some(Box::new(Expr(None)))))));
            assert_eq!(e.eval(), 3, "{}", stringify!($m));
        }};
    }
    check!(ranked_announced_self_path);
    check!(structural_announced_self_path);
}
