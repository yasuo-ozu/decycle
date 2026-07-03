//! Regression tests for the LEGACY-machinery findings fixed in this pass
//! (process_module.rs / process_trait.rs / helper.rs / lib.rs / the LEGACY regions of
//! finalize.rs). Each mod is a minimal repro of exactly one finding; see the doc comment
//! on each mod for the finding id.
//!
//! Tests live in a `#[cfg(test)] mod tests { use super::*; ... }` nested DIRECTLY inside
//! the `#[decycle::decycle]` module (matching every other test file in this crate) rather
//! than importing the trait from an outer wrapper module: the generated inductive-step
//! impls glob-import two levels up (`use super::super::*;`, reaching whatever contains
//! the `#[decycle]` module) precisely so the ranked trait wins over anything at that
//! outer level — a trait `use` placed in an outer wrapper would leak back in there and
//! create a genuine (unrelated) method-resolution ambiguity.

// `mut_pattern_param` (L-M8) below writes `mut n: u32` deliberately, to exercise a
// param pattern that used to fail to compile at all; the macro also emits a delegate
// copy of that signature (`mut` preserved verbatim, since `ImplItemFn` attrs — where an
// inner `#[allow]` would otherwise go — aren't threaded through codegen) whose body only
// forwards `n` instead of mutating it, so clippy sees an unused `mut` on that generated
// copy. File-level rather than item-level because the generated copy isn't the
// annotated item.
#![allow(unused_mut)]

/// L-C1: `#[decycle] use super::T as R;` used to silently DELETE impls of the renamed
/// trait (nothing inside the consuming module ever matched the trait by its ORIGINAL
/// name again). `finalize` now carries the local alias and renames the incoming
/// `ItemTrait` before indexing by ident.
mod rename_tracking {
    #[decycle::decycle]
    pub trait RenameBase {
        fn outer(&self) -> u32;
    }

    #[decycle::decycle(support_infinite_cycle = false)]
    pub mod inner {
        #[decycle]
        use super::RenameBase as Renamed;

        #[decycle]
        pub trait Helper {
            fn inner(&self) -> u32;
        }

        pub struct A;
        pub struct B;
        pub struct C;

        // Impl of the RENAMED trait — referenced by nothing else inside the module.
        impl Renamed for C {
            fn outer(&self) -> u32 {
                99
            }
        }

        impl Helper for B
        where
            A: Helper,
        {
            fn inner(&self) -> u32 {
                A.inner() + 39
            }
        }

        impl Helper for A {
            fn inner(&self) -> u32 {
                1
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn renamed_use_impl_is_not_dropped() {
                assert_eq!(C.outer(), 99);
                assert_eq!(B.inner(), 40);
            }
        }
    }
}

/// L-M1: `remove_cyclic_bounds` used to strip EVERY multi-segment where-bound
/// (`where Self: ::core::fmt::Debug` would vanish -> E0277), not just the cyclic one.
// `support_infinite_cycle = false`: the (unrelated) infinite-cycle re-entry machinery
// hoists a bare `Self`-mentioning where-bound onto a free fn, where `Self` doesn't
// exist — orthogonal to what this test is about (L-M1's `remove_cyclic_bounds` fix).
#[decycle::decycle(support_infinite_cycle = false)]
mod multi_segment_where_bound {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
        Self: ::core::fmt::Debug,
    {
        fn step(&self, n: u32) -> u32 {
            // Actually uses the multi-segment bound: if it were stripped from the
            // generated impls, this wouldn't compile (E0277).
            let _ = format!("{:?}", self);
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

    impl ::core::fmt::Debug for A {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            write!(f, "A")
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn multi_segment_where_bound_survives() {
            assert_eq!(A.step(4), 4);
        }
    }
}

/// L-M2: `process_trait_item_for_ranked` used to unconditionally strip default method
/// bodies (turning every defaulted method abstract on the ranked trait), so an impl
/// that relied on the trait's default hit E0046.
#[decycle::decycle]
mod defaulted_trait_method {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
        fn double_step(&self, n: u32) -> u32 {
            self.step(n) * 2
        }
    }

    pub struct A;
    pub struct B;

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
        // `double_step` intentionally NOT overridden.
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn defaulted_method_falls_back_to_default() {
            assert_eq!(A.step(4), 4);
            assert_eq!(A.double_step(4), 8);
        }
    }
}

/// L-M5(a): a `self::`-qualified reference to a `#[decycle]` trait — in the impl's trait
/// path, in a where-bound, and in a `<T as self::Trait>::method` qself value path — is
/// now normalized the same as the bare name (previously left unranked / hit the shadow).
#[decycle::decycle]
mod self_qualified {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl self::Loop for A
    where
        B: self::Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                <B as self::Loop>::step(&B, n - 1) + 1
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn self_qualified_impl_bound_and_qself_work() {
            assert_eq!(A.step(4), 4);
        }
    }
}

/// L-M5(b): a two-segment `Trait::method(...)` value path (no qself) is now rewritten to
/// the ranked trait too — previously only single-segment / qself forms were, so this
/// resolved to the shadow dummy trait (E0599/wrong dispatch).
#[decycle::decycle]
mod two_segment_trait_method {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                Loop::step(&B, n - 1) + 1
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
                Loop::step(&A, n - 1) + 1
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn two_segment_trait_method_call_is_rewritten() {
            assert_eq!(A.step(4), 4);
        }
    }
}

/// L-M7: the temporal macro name used to be keyed purely on a crate-wide deterministic
/// hash, so two `#[decycle] trait Foo` items (different modules, same name) collided on
/// the `#[macro_export]` carrier name (E0428). A per-item discriminant is now folded in.
mod duplicate_trait_name {
    mod dup_a {
        #[decycle::decycle]
        pub mod m {
            #[decycle]
            pub trait Foo {
                fn a(&self) -> u32;
            }
            pub struct X;
            pub struct Y;
            impl Foo for X
            where
                Y: Foo,
            {
                fn a(&self) -> u32 {
                    1
                }
            }
            impl Foo for Y
            where
                X: Foo,
            {
                fn a(&self) -> u32 {
                    X.a() + 1
                }
            }
        }
    }

    mod dup_b {
        #[decycle::decycle]
        pub mod m {
            #[decycle]
            pub trait Foo {
                fn b(&self) -> u32;
            }
            pub struct X;
            pub struct Y;
            impl Foo for X
            where
                Y: Foo,
            {
                fn b(&self) -> u32 {
                    10
                }
            }
            impl Foo for Y
            where
                X: Foo,
            {
                fn b(&self) -> u32 {
                    X.b() + 1
                }
            }
        }
    }

    use dup_a::m::Foo as _;
    use dup_b::m::Foo as _;

    #[test]
    fn duplicate_trait_names_across_modules_compile() {
        assert_eq!(dup_a::m::X.a(), 1);
        assert_eq!(dup_a::m::Y.a(), 2);
        assert_eq!(dup_b::m::X.b(), 10);
        assert_eq!(dup_b::m::Y.b(), 11);
    }
}

/// L-M8: `mut n: u32` (and other by-value ident patterns with `mut`) in an impl method
/// param used to be quoted whole into a call-ARGUMENT position (`f(mut n - 1)`) by
/// `variable()`, which is invalid expression syntax. `variable()` now emits only the
/// bare ident; `reduce_pat` drops `by_ref`/subpatterns but keeps `mut` on the signature.
// `support_infinite_cycle = false`: exercises specifically the DELEGATE impl's call, built via
// `input.variable()`. The unbounded-mode (`support_infinite_cycle = true`, the default)
// machinery had the identical pattern-vs-ident mistake at two OTHER call-argument sites (the
// leaf floor's `fn_call_args`, and the full-height re-entry fn's own forwarding call) — now
// fixed the same way; see the `mut_pattern_param_unbounded` twin below.
#[decycle::decycle(support_infinite_cycle = false)]
mod mut_pattern_param {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self, mut n: u32) -> u32 {
            n += 0;
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
        fn step(&self, mut n: u32) -> u32 {
            n += 0;
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn mut_pattern_param_compiles_and_runs() {
            assert_eq!(A.step(4), 4);
        }
    }
}

/// Default-mode (`support_infinite_cycle = true`) twin of `mut_pattern_param` above: the
/// unbounded-reentry machinery had the SAME pattern-vs-ident mistake at two more
/// call-argument-position sites — `emit_impl_items_leaf`'s `fn_call_args` (built via a raw
/// `quote!(#pat)` instead of `FnArgScheme::variable()`, so the floor emitted
/// `__dcl_f(mut n)`) and `normalize_reentry_sig`'s params reused as the full-height re-entry
/// fn's own forwarding call (`<S as Trait>::step(mut n)`) — both invalid expression syntax.
/// Recurses well past the default `recurse_level` (10) so the test actually crosses the floor
/// and exercises both fixed sites, not just the leaf/re-entry fn signatures (where `mut` is
/// valid syntax and was never the problem).
#[decycle::decycle]
mod mut_pattern_param_unbounded {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
    {
        fn step(&self, mut n: u32) -> u32 {
            n += 0;
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
        fn step(&self, mut n: u32) -> u32 {
            n += 0;
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn mut_pattern_param_unbounded_compiles_and_runs_past_floor() {
            assert_eq!(A.step(25), 25);
        }
    }
}

/// Default-mode hoisted-registration `Self`-bound fix: a preserved non-cyclic bound
/// mentioning bare `Self` (`where Self: ::core::fmt::Debug` — kept verbatim by
/// `remove_cyclic_bounds`, since it isn't the cyclic bound being stripped) used to be threaded
/// unsubstituted onto the per-impl hoisted `__dcl_register_once_*` FREE fn's generics in
/// `support_infinite_cycle = true` (the default) mode. A free fn has no `Self` in scope, so
/// this was a clean E0411 for any decycle cycle carrying such a bound, regardless of whether
/// the cycle ever actually recursed. `Self` is now substituted for the impl's own self type
/// (`subst_bare_self_in_generics`) before that where-clause is threaded onto the hoisted fn.
#[decycle::decycle]
mod self_bound_hoisted_registration {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Loop for A
    where
        B: Loop,
        Self: ::core::fmt::Debug,
    {
        fn step(&self, n: u32) -> u32 {
            // Actually uses the bound: if it were dropped instead of substituted, this test
            // wouldn't distinguish that from simply deleting the bound.
            let _ = format!("{:?}", self);
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

    impl ::core::fmt::Debug for A {
        fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
            write!(f, "A")
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn self_bound_compiles_and_runs_past_floor() {
            // Also recurse well past the default `recurse_level` (10): the fix must not just
            // let this compile, it must keep the floor's registration/re-entry working.
            assert_eq!(A.step(25), 25);
        }
    }
}

/// L-m1: `emit_impl_items_delegate`'s GAT case used to drop the GAT's own params
/// (`type Assoc<T2> = path::Assoc;` instead of `path::Assoc<T2>`), defaulting/miscompiling
/// the instantiation. The RHS now threads the same params through.
#[decycle::decycle]
mod gat_delegation {
    #[decycle]
    pub trait Container {
        type Item<T2>;
        fn wrap<T2>(&self, value: T2) -> Self::Item<T2>;
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl Container for A
    where
        B: Container,
    {
        type Item<T2> = Option<T2>;
        fn wrap<T2>(&self, value: T2) -> Self::Item<T2> {
            Some(value)
        }
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                B.step(n - 1) + 1
            }
        }
    }

    impl Container for B
    where
        A: Container,
    {
        type Item<T2> = Option<T2>;
        fn wrap<T2>(&self, value: T2) -> Self::Item<T2> {
            Some(value)
        }
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn gat_delegation_keeps_own_params() {
            assert_eq!(A.wrap(5u32), Some(5u32));
            assert_eq!(A.step(3), 3);
        }
    }
}
