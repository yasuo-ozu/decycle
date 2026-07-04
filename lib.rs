#![doc(html_logo_url = "https://raw.githubusercontent.com/yasuo-ozu/decycle/main/assets/logo.svg")]
#![doc(html_favicon_url = "https://raw.githubusercontent.com/yasuo-ozu/decycle/main/assets/logo.svg")]
#![doc = include_str!("README.md")]

#[doc(hidden)]
pub use decycle_macro::__finalize;

/// Low-level helper for macro crates that want to wrap `#[decycle]` on modules.
///
/// This re-export exists for bridging: a macro crate can provide its own attribute/derive
/// macros, while still delegating `#[decycle]`-style module processing to decycle.
/// For example, a library might generate trait impls via a custom macro, but still
/// want the enclosing module to be processed by decycle to break trait cycles.
pub use decycle_impl::process_module;
/// Attribute macro that expands a module or trait to break circular trait
/// obligations within the annotated module. Also see module-level documentation.
///
/// ```rust
/// # use decycle::decycle;
/// // This annotation is required to be used within #[decycle] module
/// #[decycle]
/// trait A {
///     fn a(&self) -> ::core::primitive::usize;
/// }
///
/// #[decycle]
/// mod cycle {
///     // Trait defined out of the module
///     #[decycle]
///     use super::A;
///
///     // Direct definition
///     #[decycle]
///     trait B {
///         fn b(&self) -> usize;
///     }
///
///     struct Left(usize);
///     struct Right(usize);
///
///     impl A for Left
///     where
///         Right: B,
///     {
///         fn a(&self) -> usize {
///             self.0 + 1
///         }
///     }
///
///     impl B for Right
///     where
///         Left: A,
///     {
///         fn b(&self) -> usize {
///             self.0 + 1
///         }
///     }
/// }
/// # fn main() {}
/// ```
///
/// ## Attribute Arguments
///
/// - **Module**:
///   - `#[decycle::decycle(recurse_level = N, support_infinite_cycle = true|false, decycle = path)]`
///   - `recurse_level`: expansion depth (default 10, must be at least 1)
///   - `support_infinite_cycle`: enables/disable infinite cycle handling (default true)
///   - `decycle`: override the path used to refer to this crate
/// - **Trait** (defined out of `#[decycle]` module):
///   - `#[decycle::decycle(marker = path, decycle = path)]`
///   - `marker`: marker type used for internal references. Required when the
///     trait definition contains non-absolute type paths so decycle can intern
///     them into a stable, globally reachable form.
///   - `decycle`: override the path used to refer to this crate
///
/// ### Impl where-clause bounds
/// In `impl` blocks inside a `#[decycle]` module, avoid constraining
/// `#[decycle]` traits on non-local bounded types with associated constraints.
/// For example, this is rejected:
///
/// ```rust,compile_fail
/// # use decycle::decycle;
/// #[decycle]
/// pub trait MyTrait<'a> {
///     type Assoc;
/// }
///
/// #[decycle]
/// mod m {
///     #[decycle]
///     use super::MyTrait;
///
///     pub struct MyStruct<T>(::core::marker::PhantomData<T>);
///
///     impl<'a, T> MyTrait<'a> for MyStruct<T>
///     where
///         (): MyTrait<'a, Assoc = T>,
///     {
///         type Assoc = T;
///     }
/// }
/// # fn main() {}
/// ```
///
/// Prefer `Self` or one of the `impl`'s own type parameters as the bounded
/// type in such constraints.
///
///
/// ### Recursion limits
/// `recurse_level` (must be at least 1) limits how many expansion stages are
/// used to break the cycle at compile time. What happens once real recursion
/// runs deeper than that depends on `support_infinite_cycle`:
///
/// - `support_infinite_cycle = true` (the default) does **not** stop at
///   `recurse_level`: the deepest compile-time stage (the "floor") re-enters
///   the *original* trait impl at full height through a type-erased fn
///   pointer held in a **thread-local** registry (keyed by a generated
///   per-(trait, method, instantiation) marker type plus a layout
///   fingerprint). Every inductive frame idempotently registers the
///   re-entry fns it and its cyclic-bound siblings need before descending,
///   so the floor's lookup always finds its target on the thread that needs
///   it. Recursion depth is then bounded only by the OS stack, like any
///   ordinary recursive-descent code — which also means a genuinely
///   non-terminating cycle overflows the stack instead of being cut off.
///   Three floors intentionally fail closed with an actionable, isolated
///   panic instead of silently misbehaving: (a) a generic method's floor
///   reached before any frame of that exact instantiation ran on the
///   current thread (e.g. a first descent at cycle width greater than
///   `recurse_level`); (b) any floor of an impl whose cyclic bound targets
///   a bare type parameter (`impl<T: Cb> Ca for Wrap<T>` — its re-entry
///   registration is not expressible, so the cycle is unbounded only
///   through its other impls); and (c) a heterogeneous side-bound cycle
///   where the registering impl's own bounds don't cover every bound a
///   reachable sibling impl needs (its registration is skipped rather than
///   risk naming an unprovable obligation). Such impls simply never
///   register, compile cleanly, and panic (rather than corrupt memory) if
///   their floor is ever actually reached.
/// - `support_infinite_cycle = false` emits no runtime machinery at all
///   (zero-cost) and instead stops with an `unimplemented!` panic once
///   `recurse_level` is reached.
///
/// ## Example with markers
/// Use `marker` when the trait contains non-absolute paths (e.g. `super::Type`,
/// `crate::Type`, or local aliases) so decycle can intern those references.
/// The path given with `marker = <path>` argument should be practically absolute and accessible from anywhere
/// where the defined trait is used.
///
/// ```rust
/// #[decycle::decycle(marker = Marker)]
/// trait MyTrait {
///     fn value(&self) -> i32;
/// }
/// struct Marker;
/// ```
pub use decycle_macro::decycle;

/// Low-level helper for macro crates that want to wrap `#[decycle]` on traits.
///
/// This is useful when another macro crate defines or derives traits, and those traits
/// should also be valid targets for `#[decycle]`. The wrapper macro can call into this
/// function to apply decycle's transformation while keeping its own macro API.
///
/// Requires the (default-on) `type-leak` feature; a `finalize`-only consumer that builds with
/// `default-features = false` does not get this re-export (see the `type-leak` feature doc).
#[cfg(feature = "type-leak")]
pub use decycle_impl::process_trait;

/// Programmatic entry point for the `#[decycle]` transformation.
///
/// A wrapper macro crate (e.g. syan's `#[recurse]`) constructs [`finalize::FinalizeArgs`]
/// directly and calls [`finalize::finalize`], bypassing the token-carrier ping-pong entirely
/// (and its `crate_version` assertion, which only guards the `Parse` carrier path). This
/// surface is **semver-committed**: `FinalizeArgs`' fields and `finalize`'s signature are part
/// of the public API.
///
/// (The macro ping-pong protocol used by `#[decycle]` itself also delegates here, `Parse`ing
/// `FinalizeArgs` from the specific token shape the generated carrier macros feed back into
/// `__finalize`; that path is unaffected by un-hiding this module.)
pub use decycle_impl::finalize;

/// D1 bridge (impl-spec §C.4): convenience root re-export of
/// [`decycle_impl::finalize::ranked_trait_name`] — the exact ident mangling `finalize` uses for
/// a `#[decycle]` trait's synthesized ranked counterpart. A programmatic caller (e.g. syan's
/// `#[recurse]`) needs this to spell a rank-PRESERVING wrapper impl BEFORE calling `finalize`,
/// since such a wrapper must be emitted outside `finalize`'s own output (see
/// [`finalize::AlsoRank`]'s docs on the rank-preserving wrapper constraint, and
/// [`finalize::ranked_trait_path`] for the full path form). This surface is
/// **semver-committed** alongside `finalize`/`FinalizeArgs`.
pub use decycle_impl::finalize::ranked_trait_name;

/// D1 bridge: convenience root re-export of [`decycle_impl::finalize::ranked_trait_path`] — the
/// full path to a trait's ranked counterpart as seen from the scope a rank-preserving wrapper
/// must be emitted into (a sibling of `finalize`'s own `shadowing_module`). See
/// [`ranked_trait_name`] and [`finalize::ranked_trait_path`] for details and a usage example.
pub use decycle_impl::finalize::ranked_trait_path;

/// D1 bridge (E3 replan §1.2): rank encoding + registration emitters, re-exported like
/// [`ranked_trait_name`]/[`ranked_trait_path`]. **Semver-committed.**
pub use decycle_impl::finalize::{
    emit_registration, fingerprint_expr, floor_rank, initial_rank, is_syntactically_unsized,
    method_is_generic, rank_succ, reentry_alias_name, reentry_fn_name, reentry_marker_name,
};

/// Internal helper used by generated code to track staged type expansion.
#[doc(hidden)]
pub trait Repeater<const RANDOM: u64, const IX: usize, PARAM: ?Sized> {
    /// The resolved type at the given stage.
    type Type: ?Sized;
}

/// Runtime fn-pointer registry backing unbounded `support_infinite_cycle` re-entry.
///
/// The generated rank floor, instead of erroring, re-enters the original trait impl at full
/// height through a type-erased fn pointer stored here. Keys are `type_name::<K>()` *string
/// content* of generated per-(trait, method, instantiation) marker ZSTs — robust against
/// linker identical-code-folding and `-Zshare-generics` (string identity, not address
/// identity) — paired with a layout fingerprint (`type_name` is non-injective: e.g. two
/// closures declared in one fn share a `{{closure}}` name, so the fold over each key type's
/// size/align keeps different-layout instantiations on distinct keys — an ABI-mismatched
/// transmute-call is thereby unreachable). The map is **thread-local**: every registration a
/// floor depends on is emitted on the same call stack (register-before-descend), hence the
/// same thread — so a thread-local map preserves the coverage guarantee while making
/// cross-thread interleaving physically unable to cross-contaminate keys, and there is no
/// lock to poison and no contention. Registration is an idempotent insert: the same key
/// always maps to the same fn, so overwrite is harmless.
///
/// **Semver-committed bridge surface (D1, E3 replan §1.2).** `FP_SEED`, `fp_fold`,
/// `fp_fold_word`, `register`, and `lookup` — together with the key construction
/// `(type_name::<Mk<Target, targs…, margs…>>(), fp)`, where `Mk` is the per-(trait × method)
/// marker [`decycle_impl::finalize::reentry_marker_name`] names and `fp` is
/// [`decycle_impl::finalize::fingerprint_expr`]'s fold — are a stable library API: a
/// programmatic `finalize` caller (syan's `#[recurse]`) hand-emits registrations that a
/// `finalize`-emitted floor must find. Any change to the key construction, the fold
/// constants, or these signatures is a breaking change to such callers, `__` prefix
/// notwithstanding.
pub mod __reentry {
    use std::any::type_name;
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static REG: RefCell<HashMap<(&'static str, u64), usize>> = RefCell::new(HashMap::new());
    }

    /// FNV-1a offset basis: the seed of every generated fingerprint fold.
    pub const FP_SEED: u64 = 0xcbf29ce484222325;

    const FP_PRIME: u64 = 0x100000001b3;

    /// Fold one type's layout into a fingerprint (FNV-style, deterministic).
    pub const fn fp_fold(acc: u64, size: usize, align: usize) -> u64 {
        let acc = (acc ^ (size as u64)).wrapping_mul(FP_PRIME);
        (acc ^ (align as u64)).wrapping_mul(FP_PRIME)
    }

    /// Fold one const-generic value (cast to `u64`; wider values truncate) into a fingerprint.
    pub const fn fp_fold_word(acc: u64, w: u64) -> u64 {
        (acc ^ w).wrapping_mul(FP_PRIME)
    }

    /// Register the full-height re-entry fn (as `fn`-pointer-cast-to-`usize`) for key
    /// `(K, fp)` on this thread.
    pub fn register<K: ?Sized>(fp: u64, f: usize) {
        REG.with(|reg| reg.borrow_mut().insert((type_name::<K>(), fp), f));
    }

    /// Look up the re-entry fn for key `(K, fp)`, copied out as `usize`. The value is copied
    /// and the `RefCell` borrow released before the not-registered panic can fire.
    pub fn lookup<K: ?Sized>(fp: u64) -> usize {
        let found = REG.with(|reg| reg.borrow().get(&(type_name::<K>(), fp)).copied());
        found.expect(
            "decycle: re-entry fn not registered before the floor was reached. This floor's \
             key had no same-instantiation frame run on this thread's descent first — e.g. a \
             generic method's first descent at cycle width > recurse_level (including \
             self-recursion consuming ranks before the first generic cross-edge call), or an \
             impl whose cyclic bound targets a bare type parameter. Increase recurse_level; \
             if two same-signature closures are involved, give them distinct named types.",
        )
    }
}

#[doc(hidden)]
pub use decycle_impl::proc_macro_error;
#[cfg(feature = "type-leak")]
#[doc(hidden)]
pub use decycle_impl::type_leak;
