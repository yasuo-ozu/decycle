#![doc(html_logo_url = "https://raw.githubusercontent.com/yasuo-ozu/decycle/main/decycle.png")]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/yasuo-ozu/decycle/main/decycle.png"
)]
#![doc = include_str!("README.md")]

#[doc(hidden)]
pub use decycle_macro::__finalize;

/// The default **ranked** engine's programmatic API — hidden "Ranked" helper traits plus a
/// thread-local re-entry registry. For macro authors and tooling built on decycle; most users want
/// the [`macro@decycle`] attribute. See the module for its entry points.
pub use decycle_impl::ranked;
/// The **structural** unroll engine's programmatic API (`#[decycle(structural)]`) — a compile-time
/// unroll with no runtime and no `type-leak` dependency.
pub use decycle_impl::structural;
/// Attribute macro that expands a module or trait to break circular trait
/// obligations within the annotated module. Also see module-level documentation.
///
/// # Two engines
///
/// On a module, `#[decycle]` selects one of two independent cycle-breaking strategies:
///
/// - the default **ranked** engine (`#[decycle]`) — hidden "Ranked" helper traits plus a
///   thread-local runtime re-entry registry;
/// - the **structural** unroll (`#[decycle(structural)]`) — per-type `#[repr(transparent)]`
///   terminators and layout casts, with no runtime and no `type-leak` dependency.
///
/// They break the same cycles and are interchangeable for ordinary method recursion, but differ
/// in cost and in what each can't do (structural is zero-cost and fails every unsupported shape at
/// compile time; ranked is more expressive for growing-type-argument recursion but a residual set
/// of shapes fails closed at runtime). See the crate README's *Two algorithms* section for the full
/// per-engine matrix. Both engines require the whole cycle in one *inline* `#[decycle]` module over
/// traits **you** annotate — either `#[decycle]` on the trait definition, or a `#[decycle] use` of a
/// trait defined elsewhere.
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
///   - `#[decycle::decycle(structural, recurse_level = N, support_infinite_cycle = true|false, decycle = path)]`
///   - `structural`: select the structural unroll instead of the ranked engine (see *Two engines*
///     above); incompatible with `recurse_level` / `support_infinite_cycle`
///   - `recurse_level`: expansion depth (default 10, must be at least 1); ranked engine only
///   - `support_infinite_cycle`: enable/disable infinite-cycle handling (default true); ranked engine only
///   - `decycle`: override the path used to refer to this crate
///
///   An empty `#[decycle]` module is rejected by both engines. A module with no `#[decycle]`-annotated
///   cycle is rejected by the ranked engine and treated as a silent no-op by the structural engine.
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
/// ## Unsupported shapes
/// Both engines reject these up-front, with an actionable error on the offending item:
///
/// - `async fn` — return a boxed future (`-> Pin<Box<dyn Future<Output = ..>>>`) instead;
/// - return-position `impl Trait` — declare an associated type and return it
///   (`type Output; fn m(&self) -> Self::Output`), or return a concrete/boxed type;
/// - a *wrapped* cyclic bound `Box<Stmt>: Tr` without a blanket `impl<T: Tr> Tr for Box<T>`;
/// - a cross-module cycle, or a foreign trait implemented for a *foreign* type.
///
/// An `unsafe trait` **is** supported (the generated impls are emitted as `unsafe impl`). A
/// third-party trait may participate in the cycle when brought in with `#[decycle] use` and
/// implemented for your own types. For the complete per-engine matrix — growing-type-argument
/// recursion, `#[track_caller]`, `no_std`, higher-order `Self`-mentioning arguments
/// (`fn(&Self)` / `dyn` / `impl Fn(&Self)`), and more — see the crate README's *Two algorithms*
/// section.
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

/// Internal helper used by generated code to track staged type expansion.
#[doc(hidden)]
pub trait Repeater<const RANDOM: u64, const IX: usize, PARAM: ?Sized> {
    /// The resolved type at the given stage.
    type Type: ?Sized;
}

/// Runtime fn-pointer registry backing unbounded `support_infinite_cycle` re-entry.
#[doc(hidden)]
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
             if two same-LAYOUT closures share this method's floor (even with different \
             signatures or bodies — `type_name` collapses closures and the key folds only \
             layout), give them distinct named types.",
        )
    }
}

#[doc(hidden)]
pub use decycle_impl::proc_macro_error;
#[doc(hidden)]
pub use decycle_impl::type_leak;
