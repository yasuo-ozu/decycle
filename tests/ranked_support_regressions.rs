//! Regressions for the ranked engine's *support* passes — the ones that run before `finalize` and
//! whose defects were silent rather than loud.
//!
//! Every case here asserts a real value: each was a program that either compiled to the WRONG
//! answer, was rejected though it is valid Rust, or panicked the proc macro outright.

/// **`super::`-rooted paths in VALUE position must keep meaning what the caller wrote.**
///
/// `finalize` re-emits every adopted impl one or two modules deeper, so a relative path written in
/// the body no longer points where it did. `LiftRelative` lifted types, trait bounds and the trait
/// half of a qualified path, but left plain expression paths (and struct-literal / pattern paths)
/// "to resolve normally" — and they DO resolve, to a different item: `super::` now names the
/// processed module itself, which glob-imports everything the real parent had. Where the module
/// defines its own `helper`, `super::helper()` silently returned **1** where **100** is correct.
mod super_value_paths {
    pub fn helper() -> usize {
        100
    }

    pub const SEED: usize = 1000;

    pub struct Outer {
        pub n: usize,
    }
    impl Outer {
        pub fn weight(&self) -> usize {
            self.n * 3
        }
    }

    // `Shadow` exists only so the `match` in the body below has an arm that must NOT be taken.
    #[allow(dead_code)]
    pub enum Tag {
        Real,
        Shadow,
    }
    pub fn make_tag() -> Tag {
        Tag::Real
    }

    #[decycle::decycle]
    // The shadows below are deliberately unreachable once the paths resolve correctly.
    #[allow(dead_code)]
    pub mod m {
        #[decycle]
        pub trait Tr {
            fn f(&self) -> usize;
        }

        // Every name the bodies reach for through `super::` also exists HERE, one level too
        // shallow — which is exactly what made the mis-resolution silent instead of an error.
        // Each shadow answers DIFFERENTLY, so a wrong resolution changes the value rather than
        // failing to compile.
        pub fn helper() -> usize {
            1
        }
        pub const SEED: usize = 2;
        pub struct Outer {
            pub n: usize,
        }
        impl Outer {
            pub fn weight(&self) -> usize {
                self.n
            }
        }
        pub enum Tag {
            Real,
            Shadow,
        }
        pub fn make_tag() -> Tag {
            Tag::Shadow
        }

        pub struct A(pub Box<B>);
        pub struct B(pub Option<Box<A>>);

        impl Tr for A
        where
            B: Tr,
        {
            fn f(&self) -> usize {
                // A plain expression path (a free fn) and a const, both `super::`-rooted.
                super::helper() + super::SEED + self.0.f()
            }
        }

        impl Tr for B
        where
            A: Tr,
        {
            fn f(&self) -> usize {
                // A struct-literal path, and a pattern path matched against a value produced by
                // another `super::`-rooted call: all three have to name the SAME (outer) items.
                let outer = super::Outer { n: 10 };
                let base = match super::make_tag() {
                    super::Tag::Real => outer.weight(),
                    super::Tag::Shadow => 0,
                };
                base + self.0.as_ref().map(|a| a.f()).unwrap_or(0)
            }
        }
    }

    #[test]
    fn super_rooted_value_paths_name_the_real_parent() {
        use m::Tr as _;
        // Correct: helper() = 100, SEED = 1000, `Outer{n:10}.weight()` = 30, `make_tag()` = Real.
        // Mis-resolved to the module's own items it was 1 + 2 + 0 instead.
        assert_eq!(m::B(None).f(), 30);
        assert_eq!(m::A(Box::new(m::B(None))).f(), 100 + 1000 + 30);
        assert_eq!(
            m::B(Some(Box::new(m::A(Box::new(m::B(None)))))).f(),
            30 + 1130
        );
    }
}

/// **A foreign bound target must not join a local sibling's premise group.**
///
/// `sharing_sources` identified a cyclic where-bound's target by its LAST path segment, so the
/// premise `crate::…::other::B: Tr` — a bound on a type defined outside the module — read as the
/// module's own member `B` and fabricated the obligation edge `A -> B`. Predicate sharing then
/// unioned `B`'s unrelated leaf premises into `A`'s impl, and `A<NotClone>: Tr` stopped holding
/// (`E0599 … the trait bound NotClone: Clone is not satisfied`) — while renaming the foreign type
/// from `other::B` to `other::C` made the byte-identical program compile.
mod foreign_bound_target {
    pub mod other {
        pub struct B;
    }

    #[decycle::decycle]
    pub mod m {
        #[decycle]
        pub trait Tr {
            fn f(&self) -> usize;
        }

        pub struct A<T>(pub T);
        pub struct B<T>(pub T);

        // `A` is its own cycle. The second bound is an ordinary premise on a FOREIGN type that
        // merely shares its last segment with the sibling member `B`.
        impl<T> Tr for A<T>
        where
            A<T>: Tr,
            crate::foreign_bound_target::other::B: Tr,
        {
            fn f(&self) -> usize {
                1
            }
        }

        // The sibling, whose `T: Clone` must stay put: nothing obligates `A` to prove it.
        impl<T> Tr for B<T>
        where
            T: Clone,
        {
            fn f(&self) -> usize {
                2
            }
        }
    }

    impl m::Tr for other::B {
        fn f(&self) -> usize {
            7
        }
    }

    /// Deliberately not `Clone`: this is what the injected premise rejected.
    pub struct NotClone;

    #[test]
    fn foreign_target_does_not_inject_a_siblings_premise() {
        use m::Tr as _;
        assert_eq!(m::A(NotClone).f(), 1);
        assert_eq!(m::B(1u8).f(), 2);
        assert_eq!(other::B.f(), 7);
    }
}

/// **Raw identifiers.** Every generated name is built by interpolating a user ident into a
/// `format!`, and `Ident::new` panics on the `#` of a raw identifier — so a cycle head named
/// `r#loop` inside a module named `r#match` used to abort the whole compilation with
/// `custom attribute panicked: "__DecycleNat_r#loop_m" is not a valid identifier`, with no span
/// and no mention of what was wrong. Raw identifiers are ordinary Rust; nothing about them is
/// unsupported.
mod raw_identifiers {
    #[decycle::decycle]
    #[allow(non_camel_case_types)]
    pub mod r#match {
        #[decycle]
        pub trait Tr {
            fn depth(&self) -> usize;
        }

        pub struct r#loop(pub Box<r#struct>);
        pub struct r#struct(pub Option<Box<r#loop>>);

        impl Tr for r#loop
        where
            r#struct: Tr,
        {
            fn depth(&self) -> usize {
                self.0.depth() + 1
            }
        }

        impl Tr for r#struct
        where
            r#loop: Tr,
        {
            fn depth(&self) -> usize {
                self.0.as_ref().map(|l| l.depth()).unwrap_or(0) + 1
            }
        }
    }

    #[test]
    fn raw_ident_cycle_head_and_module() {
        use r#match::Tr as _;
        let leaf = r#match::r#struct(None);
        assert_eq!(leaf.depth(), 1);
        let nested = r#match::r#loop(Box::new(r#match::r#struct(Some(Box::new(
            r#match::r#loop(Box::new(r#match::r#struct(None))),
        )))));
        assert_eq!(nested.depth(), 4);
    }
}
