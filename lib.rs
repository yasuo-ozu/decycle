#![cfg_attr(not(feature = "std"), no_std)]
#![doc(html_logo_url = "https://raw.githubusercontent.com/yasuo-ozu/decycle/main/decycle.png")]
#![doc(
    html_favicon_url = "https://raw.githubusercontent.com/yasuo-ozu/decycle/main/decycle.png"
)]
#![doc = include_str!("README.md")]

#[doc(hidden)]
pub use decycle_macro::__finalize;

/// Re-export of the [`announce`](https://docs.rs/announce) crate, so that a crate can announce its
/// traits for `#[decycle] use` without depending on `announce` itself:
/// `#[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]`. The option
/// `crate = ..` tells `#[announce]` where its runtime is; it is optional for a crate that depends on
/// `announce` directly. Needs the `announce` feature (on by default); it is built without `std`, so a
/// `#![no_std]` crate can announce a trait. See [`check`].
#[cfg(feature = "announce")]
pub use announce;

/// The default **ranked** engine's programmatic API — hidden "Ranked" helper traits plus a
/// thread-local re-entry registry. For macro authors and tooling built on decycle; most users want
/// the [`macro@decycle`] attribute. See the module for its entry points.
#[cfg(feature = "api")]
pub use decycle_impl::ranked;
/// The **structural** unroll engine's programmatic API (`#[decycle(structural)]`) — a compile-time
/// unroll that emits no runtime machinery.
#[cfg(feature = "api")]
pub use decycle_impl::structural;
/// Engine-independent, code-free inspection: the module's obligation graph over type idents, with
/// each edge labelled `Direct` or `Peeled`.
///
/// The analysis itself is the same whichever engine you go on to use — it lives here, and only here.
/// What differs per engine is how a graph is *consumed*: [`ranked::process_module_with_graph`]
/// **replaces** its participant set with the graph's nodes, whereas
/// [`structural::process_module_with_graph`] can only **filter** with them, its model being keyed on
/// `(type, trait)` pairs rather than on types.
///
/// - [`analysis::analyze_module`] reads a module's own classification back out;
/// - [`analysis::cyclic_subgraph`] restricts a graph to the nodes that lie on a cycle, so a caller
///   can supply a whole reference relation and let decycle decide what recurses;
/// - [`analysis::with_nodes`] adds participants discovered after the graph was built.
///
/// The result is what [`ranked::process_module_with_graph`] and
/// [`structural::process_module_with_graph`] take.
#[cfg(feature = "api")]
pub use decycle_impl::analysis;
/// Re-exported so callers can name the [`safegraph::VecGraph`] that
/// [`analysis::analyze_module`] returns without depending on `safegraph` themselves.
#[cfg(feature = "api")]
pub use decycle_impl::safegraph;
/// Attribute macro that expands a module to break circular trait obligations within it. Also see
/// module-level documentation.
///
/// # Two engines
///
/// On a module, `#[decycle]` selects one of two independent cycle-breaking strategies:
///
/// - the default **ranked** engine (`#[decycle]`) — hidden "Ranked" helper traits plus a
///   thread-local runtime re-entry registry;
/// - the **structural** unroll (`#[decycle(structural)]`) — per-type `#[repr(transparent)]`
///   terminators and layout casts, emitting no runtime machinery.
///
/// They break the same cycles and are interchangeable for ordinary method recursion, but differ
/// in cost and in what each can't do (structural is zero-cost and fails every unsupported shape at
/// compile time; ranked can re-enter across several instantiations of a generic method, but a residual
/// set of shapes fails closed at runtime). See the crate README's *Two algorithms* section for the full
/// per-engine matrix. Both engines require the whole cycle in one *inline* `#[decycle]` module over
/// traits **you** annotate — either `#[decycle]` on a trait defined in the module, or a
/// `#[decycle] use` of a trait announced elsewhere (see
/// [Traits outside the module](#traits-outside-the-module)).
///
/// ```rust
/// # use decycle::decycle;
/// // A trait outside the module is announced, not annotated with `#[decycle]`.
/// #[decycle::announce::announce(crate = ::decycle::announce)]
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
///
/// ### Traits outside the module
/// A trait defined outside a `#[decycle]` module is announced with
/// [`#[announce]`](https://docs.rs/announce/latest/announce/attr.announce.html) (re-exported as
/// `decycle::announce`): that makes its definition available to any crate, and
/// `#[decycle] use path::to::Trait;` brings it into a module. `#[decycle]` on such a trait expands to
/// `#[announce(crate = ::decycle::announce)]`; write `#[announce]` itself for its options.
///
/// ```text
/// #[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
/// pub trait Trait { .. }
/// ```
///
/// The options of 0.5's `#[decycle]` on a trait map to:
///
/// | 0.5 `#[decycle(..)]` on a trait | now |
/// | --- | --- |
/// | `#[decycle]` | `#[decycle]` (expands to `#[announce(crate = ::decycle::announce)]`) |
/// | `alter_macro_name = N` | none; `#[decycle] use` the trait by its own name |
/// | `decycle = path` | unchanged; the trait is announced with `crate = path::announce` |
/// | `marker = path` | none; announce checks the trait's paths at its definition: write them absolutely (`crate::..`, `::krate::..`) |
/// | (checks at use) | `decycle::check`, see [`check`] |
///
/// ### Stand-ins for traits you do not own
/// `#[announce(mock = path::to::RealTrait)]` on a trait declares it a copy of the real trait at
/// that path (which need not be announced, and may live in another crate). announce does not emit
/// the stand-in as a trait in place: it only emits its trampoline macro, named after the stand-in,
/// at the stand-in's path and visibility. `#[decycle] use path::to::Stand;` then brings in the copy
/// as the **real** trait, so the module's impls implement `RealTrait`. Name the stand-in like the
/// real trait, or `#[decycle] use path::to::Stand as RealTrait;` when its name differs (the ranked
/// engine rejects a stand-in `use`d under another name without the `as`; the structural engine reads
/// no trait through a `use`, so it cannot tell, and the module fails with an unrelated `E0275`), and
/// spell `mock = ..` absolutely (`crate::..`, `::krate::..`).
///
/// A stand-in written by hand imports nothing but its trampoline, so the module also `use`s the
/// real trait under that name (below). [`announce-std`](https://docs.rs/announce-std)'s copies of the
/// traits of `std` work as they are: `announce_std::fmt::Display` is the real trait in the type
/// namespace as well as the trampoline, so `#[decycle] use announce_std::fmt::Display;` is all a
/// module writes, and `use core::fmt::Display;` beside it is an error (`Display` defined twice).
///
/// ```rust
/// #[decycle::decycle]
/// mod shapes {
///     #[decycle]
///     use announce_std::fmt::Display;
///
///     pub struct Pair(pub Option<Box<Tag>>);
///     pub struct Tag(pub Option<Box<Pair>>);
///
///     impl Display for Pair where Tag: Display {
///         fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
///             match &self.0 {
///                 Some(t) => write!(f, "Pair({})", t),
///                 None => write!(f, "Pair"),
///             }
///         }
///     }
///     impl Display for Tag where Pair: Display {
///         fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
///             match &self.0 {
///                 Some(p) => write!(f, "Tag({})", p),
///                 None => write!(f, "Tag"),
///             }
///         }
///     }
/// }
///
/// fn main() {
///     let p = shapes::Pair(Some(Box::new(shapes::Tag(None))));
///     assert_eq!(p.to_string(), "Pair(Tag)");
/// }
/// ```
///
/// A hand-written stand-in, and the real trait beside it:
///
/// ```rust
/// pub mod real {
///     pub trait Size { fn size(&self) -> usize; }
/// }
/// pub mod stand {
///     #[decycle::announce::announce(crate = ::decycle::announce, mock = crate::real::Size)]
///     pub trait Size { fn size(&self) -> usize; }
/// }
///
/// #[decycle::decycle]
/// mod tree {
///     use crate::real::Size;
///     #[decycle]
///     use crate::stand::Size;
///
///     pub struct Node(pub Vec<Leaf>);
///     pub struct Leaf(pub Option<Box<Node>>);
///     impl Size for Node where Leaf: Size {
///         fn size(&self) -> usize { 1 + self.0.iter().map(|l| l.size()).sum::<usize>() }
///     }
///     impl Size for Leaf where Node: Size {
///         fn size(&self) -> usize { self.0.as_ref().map_or(1, |n| n.size()) }
///     }
/// }
///
/// fn main() {
///     use real::Size;
///     let t = tree::Node(vec![tree::Leaf(None), tree::Leaf(Some(Box::new(tree::Node(vec![]))))]);
///     assert_eq!(t.size(), 3);
/// }
/// ```
///
/// ### Impl where-clause bounds
/// In `impl` blocks inside a `#[decycle]` module, avoid constraining
/// `#[decycle]` traits on non-local bounded types with associated constraints.
/// For example, this is rejected:
///
/// ```rust,compile_fail
/// # use decycle::decycle;
/// #[decycle::announce::announce(crate = ::decycle::announce)]
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
///   fingerprint). A key must name exactly one type, so a cyclic method
///   instantiated with an **anonymous type** — a closure or `async` block,
///   e.g. as an `impl Fn(..)` argument — is rejected at runtime with a
///   panic mentioning "anonymous type", deterministically on the method's
///   first call (rustc renders every closure in a function as
///   `{{closure}}`, so two of them in one function would share a key and
///   the floor would call the wrong one). Pass a named function, coerce a
///   non-capturing closure to a function pointer (`f as fn(_) -> _`), or
///   use `support_infinite_cycle = false`, which has no registry and
///   accepts closures. Every inductive frame idempotently registers the
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
/// One restriction is enforced at **runtime** instead: in the ranked engine's unbounded mode
/// (`support_infinite_cycle = true`, the default), a cyclic method instantiated with a closure or
/// `async` block panics on its first call with a message containing "anonymous type" — such types
/// cannot be encoded as unique re-entry registry keys. Pass a named function, coerce a
/// non-capturing closure to a function pointer (`f as fn(_) -> _`), or set
/// `support_infinite_cycle = false`. See *Recursion limits* above.
///
/// An `unsafe trait` **is** supported (the generated impls are emitted as `unsafe impl`). A
/// third-party trait may participate in the cycle when brought in with `#[decycle] use` and
/// implemented for your own types. For the complete per-engine matrix — multi-instantiation re-entry,
/// `#[track_caller]`, `no_std`, higher-order `Self`-mentioning arguments
/// (`fn(&Self)` / `dyn` / `impl Fn(&Self)`), and more — see the crate README's *Two algorithms*
/// section.
///
/// ## Paths in an announced trait
/// `#[announce]` hands the trait to other crates, and checks at its definition that every path in
/// it means the same thing there. Spell the paths of the trait absolutely: `crate::Base`,
/// `::core::fmt::Debug`, `crate::Local`. A path relative to the defining module (`Local`,
/// `super::X`, `std::fmt::Debug` without the leading `::`) is an error that names it. Primitive
/// types need nothing, and `allow_std_prelude = true` accepts the bare std prelude names
/// (`Clone`, `String`, `Vec<u8>`, `Option<T>`). The error also suggests announce's `repeater`;
/// `#[decycle] use` does not support it, so write the path absolutely. The same trait defined inside a `#[decycle]` module needs none of this, since it is
/// not carried through a macro.
///
/// ```rust
/// # use decycle::decycle;
/// pub struct Value(pub i32);
///
/// #[decycle::announce::announce(crate = ::decycle::announce, allow_std_prelude = true)]
/// pub trait MyTrait {
///     fn value(&self) -> crate::Value;
///     fn text(&self) -> String;
/// }
/// # fn main() {}
/// ```
pub use decycle_macro::decycle;

/// Checks a trait at its definition: it can take part in a `#[decycle]` cycle.
///
/// A trait is made available to `#[decycle] use` by announcing it with the
/// [`announce`](https://docs.rs/announce) attribute (re-exported as `decycle::announce`).
/// `#[decycle]` reads the trait through its trampoline, so nothing else is needed. Without this
/// check the trait is validated only when a `#[decycle]` module receives it, and the error appears
/// there. List `decycle::check` among the options of `#[announce]` to get the error at the trait:
///
/// ```
/// use decycle::{announce::announce, decycle};
///
/// // `crate = ..` is needed when the crate does not depend on `announce` itself.
/// #[announce(crate = ::decycle::announce, decycle::check)]
/// pub trait Walk {
///     fn walk(&self, depth: usize) -> usize;
/// }
///
/// #[decycle]
/// mod m {
///     #[decycle]
///     pub use super::Walk;
///
///     pub struct A(pub Option<Box<B>>);
///     pub struct B(pub Option<Box<A>>);
///
///     impl Walk for A where B: Walk {
///         fn walk(&self, depth: usize) -> usize {
///             self.0.as_ref().map_or(depth, |b| b.walk(depth + 1))
///         }
///     }
///     impl Walk for B where A: Walk {
///         fn walk(&self, depth: usize) -> usize {
///             self.0.as_ref().map_or(depth, |a| a.walk(depth + 1))
///         }
///     }
/// }
/// # fn main() {
/// use m::Walk;
/// assert_eq!(m::A(Some(Box::new(m::B(None)))).walk(0), 1);
/// # }
/// ```
///
/// A trait that `#[decycle]` cannot carry through a cycle is rejected at its definition:
///
/// ```compile_fail
/// use decycle::announce::announce;
/// #[announce(crate = ::decycle::announce, decycle::check)]
/// pub trait Bad {
///     fn items(&self) -> impl ::core::iter::Iterator<Item = u8>; // return-position `impl Trait`
/// }
/// ```
///
/// The rules that depend on the trait alone are checked:
///
/// - no return-position `impl Trait` in a method (`-> impl Iterator<..>`): it has no nameable type
///   for the re-entry. Declare an associated type and return it instead;
/// - no `async fn` in a method: its `impl Future` return is such a type. Return a boxed future
///   (`-> Pin<Box<dyn Future<Output = ..>>>`) instead;
/// - no default on a generic parameter (`trait Tr<T = u8>`, `trait Tr<const N: usize = 4>`): every
///   use of the trait is re-spelled with a rank argument, so each use site has to write every
///   parameter. This is a rule of the **ranked** engine only.
///
/// `decycle::check` enforces the rules of the default ranked engine, the stricter of the two. The
/// structural engine (`#[decycle(structural)]`) shares the first two and accepts a default on a
/// generic parameter, so a trait that is only meant for it is checked with `decycle::check(structural)`,
/// which skips the third rule:
///
/// ```
/// use decycle::{announce::announce, decycle};
///
/// #[announce(crate = ::decycle::announce, decycle::check(structural))]
/// pub trait Weigh<T = u8> {
///     fn weigh(&self, t: T) -> usize;
/// }
///
/// #[decycle(structural)]
/// mod m {
///     #[decycle]
///     pub use super::Weigh;
///
///     pub struct A(pub Option<Box<A>>);
///
///     // The defaulted argument may be left out under the structural engine.
///     impl Weigh for A where A: Weigh {
///         fn weigh(&self, t: u8) -> usize {
///             self.0.as_ref().map_or(t as usize, |a| a.weigh(t) + 1)
///         }
///     }
/// }
/// # fn main() {
/// use m::Weigh as _;
/// assert_eq!(m::A(Some(Box::new(m::A(None)))).weigh(5), 6);
/// # }
/// ```
///
/// The trait is the same either way: `check` only decides which engine's rules are enforced at its
/// definition, and a trait checked with `(structural)` still fails in a ranked `#[decycle]` module
/// if it has a default. Whatever depends on the impls (a where-clause bound, an `async fn` impl,
/// a `#[track_caller]`) can only be checked when the module is expanded.
pub use decycle_macro::check;

/// Runtime fn-pointer registry backing unbounded `support_infinite_cycle` re-entry.
#[doc(hidden)]
pub mod __reentry {
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

    /// The registry proper. Only this half needs `std`; the fingerprint folds above are
    /// pure `const fn` and stay available either way.
    #[cfg(feature = "std")]
    mod imp {
        use std::any::type_name;
        use std::cell::RefCell;
        use std::collections::HashMap;

        // The value is a `*const ()`, never a `usize`: a fn pointer laundered through an integer
        // loses its provenance, and calling the result of `transmute::<usize, fn(..)>` is undefined
        // behavior (miri: "pointer not dereferenceable: .. it has no provenance"). Keeping a real
        // pointer carries provenance end to end. A raw pointer is fine here because the map is
        // thread-local and never crosses a thread boundary.
        thread_local! {
            static REG: RefCell<HashMap<(&'static str, u64), *const ()>> = RefCell::new(HashMap::new());
            /// Undo log for [`Registration`]. Every registration appends what it displaced; a guard
            /// records only its own index into this log. Keeping the payload here rather than in the
            /// guard is what keeps the guard 8 bytes: a deep unbounded descent holds one guard set
            /// per live frame, and fatter guards overflow the stack well before the recursion does
            /// (20_000 frames is an ordinary depth for this engine).
            static UNDO: RefCell<Vec<Undo>> = const { RefCell::new(Vec::new()) };
        }

        type Undo = ((&'static str, u64), Option<*const ()>);


        /// Puts back the entries that were *displaced* inside one registration scope.
        ///
        /// Registration is no longer last-writer-wins. [`scope`] returns this guard, the generated
        /// prologue binds it for the rest of the method body, and dropping it restores whatever
        /// occupied those slots before — including on unwind.
        ///
        /// This matters because keys can collide: two closures written in one function share a
        /// `type_name` exactly, so two same-layout closures map to one slot. Without the restore, a
        /// nested descent started from the middle of a method body overwrites the slot, and when the
        /// outer frame reaches its floor it calls the *other* closure's fn — observably wrong
        /// results, and a segfault when the two closures' captures differ in kind.
        ///
        /// A registration that *created* its slot is deliberately left in place when the guard drops.
        /// Two documented behaviors depend on that persistence: the bare-param `impl<T: Cb> Ca for
        /// Wrap<T>` case is "unbounded once primed" (an earlier call through the Final delegating
        /// impl registers the floor a later call needs), and a descent that hit the fail-closed panic
        /// below leaves its registrations behind so a subsequent good call succeeds. Only *shadowing*
        /// is scoped; priming is not.
        #[must_use = "displaced entries are only restored when this guard drops; binding it to `_` \
                      drops it immediately and re-opens the wrong-fn window it exists to close"]
        pub struct Registration {
            /// Length of `UNDO` when this scope opened. Everything logged at or after this index
            /// belongs to the scope and is rolled back when it closes. One 8-byte guard covers a
            /// whole frame's registrations, which matters: a deep unbounded descent holds one per
            /// live frame, and per-registration guards cost enough stack to cut the reachable
            /// recursion depth by a third.
            mark: usize,
        }

        impl Drop for Registration {
            fn drop(&mut self) {
                let mark = self.mark;
                // Roll back every record at or after this guard's mark. Truncating to a mark rather
                // than popping exactly one record keeps this correct no matter what order the guards
                // are dropped in — tuple fields drop front-to-back while locals drop back-to-front,
                // and both shapes appear in generated code.
                loop {
                    // `try_with`: a decycled call can run from a TLS destructor, where the registry
                    // is already gone. Nothing to restore in that case.
                    let rec = match UNDO.try_with(|u| {
                        let mut log = u.borrow_mut();
                        if log.len() > mark {
                            log.pop()
                        } else {
                            None
                        }
                    }) {
                        Ok(Some(rec)) => rec,
                        _ => return,
                    };
                    // `None` => that registration created its slot; leave it in place (see the type
                    // docs: priming and healing depend on it). Only displacement is undone.
                    if let (key, Some(prev)) = rec {
                        let _ = REG.try_with(|reg| {
                            reg.borrow_mut().insert(key, prev);
                        });
                    }
                }
            }
        }

        /// Is this `type_name` string a **unique** name for its type?
        ///
        /// The registry keys on `type_name::<Mk<..>>()`, so a key is only sound when that string
        /// identifies exactly one type. Anonymous types break that: rustc renders every closure,
        /// `async` block, and nested closure in one function as `{{closure}}`, with no
        /// disambiguator, so two of them in the same function produce the *same* key.
        ///
        /// Names WITHOUT `{{closure}}` are unique and safe: named functions (`path::named_fn`),
        /// function pointers (`fn(usize) -> usize` — one type, hence one monomorphisation, so
        /// sharing a slot is correct), and ordinary named types.
        fn key_is_nameable(name: &str) -> bool {
            !name.contains("{{closure}}")
        }

        /// Panic if this key cannot be encoded uniquely.
        ///
        /// This fires on the FIRST registration or lookup made by a closure-instantiated method,
        /// deterministically — it does not wait for an actual collision, so the failure does not
        /// depend on which other closures happen to exist or on their layouts.
        ///
        /// It is a runtime check because a compile-time one is not expressible on stable: the
        /// natural spelling is a `const` assertion over `type_name::<K>()`, but `type_name` is not
        /// yet const-stable (`const_type_name`), and no stable alternative distinguishes two
        /// closures (`TypeId` requires `'static`; a per-monomorphisation function address is merged
        /// by identical-code folding in release builds).
        fn assert_key_encodable(key: &'static str) {
            assert!(
                key_is_nameable(key),
                "decycle: this method was instantiated with an anonymous type (a closure or `async` \
                 block), which cannot be encoded as a re-entry registry key: rustc names every \
                 closure in a function `{{{{closure}}}}`, so two of them in one function collide and \
                 the floor would call the wrong one. Pass a named function, coerce to a function \
                 pointer (`f as fn(_) -> _`), or set `support_infinite_cycle = false`. \
                 Offending key: {}",
                key
            );
        }

        /// Open a registration scope. Every [`register`] call made while the returned
        /// [`Registration`] is alive is rolled back when it drops — including on unwind. The
        /// generated method prologue opens one of these before its registrations and holds it for
        /// the rest of the body, so the entries stay live for exactly the descent they serve.
        pub fn scope() -> Registration {
            let mark = UNDO
                .try_with(|u| u.borrow().len())
                .unwrap_or(usize::MAX);
            Registration { mark }
        }

        /// Register the full-height re-entry fn for key `(K, fp)` on this thread, within the
        /// innermost open [`scope`].
        ///
        /// Call this only inside a live [`scope`]: the displacement record it logs is reclaimed when
        /// a scope closes, so registering outside one leaves a record that is never reclaimed. All
        /// generated code opens a scope first.
        pub fn register<K: ?Sized>(fp: u64, f: *const ()) {
            assert_key_encodable(type_name::<K>());
            let key = (type_name::<K>(), fp);
            let prev = REG
                .try_with(|reg| reg.borrow_mut().insert(key, f))
                .ok()
                .flatten();
            let _ = UNDO.try_with(|u| u.borrow_mut().push((key, prev)));
        }

        /// Look up the re-entry fn for key `(K, fp)`, copied out as a `*const ()`. The value is
        /// copied and the `RefCell` borrow released before the not-registered panic can fire.
        pub fn lookup<K: ?Sized>(fp: u64) -> *const () {
            assert_key_encodable(type_name::<K>());
            let found = REG
                .try_with(|reg| reg.borrow().get(&(type_name::<K>(), fp)).copied())
                .ok()
                .flatten();
            found.expect(
                "decycle: re-entry fn not registered before the floor was reached. This floor's \
                 key had no same-instantiation frame run on this thread's descent first — e.g. a \
                 generic method reached from a NON-generic caller, so no scope on the way down \
                 could name its type argument (a cross edge from a caller declaring the same \
                 generics is registered from that caller's prologue, and does not land here), or \
                 an impl whose cyclic bound targets a bare type parameter. Increase recurse_level. \
                 (This can also fire when the call runs from a thread-local destructor, after the \
                 registry for this thread has already been torn down.)",
            )
        }
    }

    /// Stand-in for the registry when the `std` feature is off.
    ///
    /// The registry is a `thread_local!` map, so unbounded re-entry cannot work without `std`.
    /// Rather than let the generated code fail with `could not find __reentry in decycle` — which
    /// names an internal module and suggests nothing — every entry point here carries a bound that
    /// cannot be met, so the error names the thing to do. Bounded mode
    /// (`support_infinite_cycle = false`) emits none of these calls and is unaffected.
    /// Never implemented. Its name is the diagnostic, and it lives here rather than inside `imp`
    /// so the error names `decycle::__reentry::…` instead of an inner module.
    ///
    /// The bound sits on `register`/`lookup`'s existing `K` parameter rather than on a concrete
    /// type, so it is checked where the generated code CALLS them. A `where (): _` bound would be
    /// a trivial bound on a concrete type and would fire here, in this crate, for every user —
    /// including the structural ones this is meant to leave alone.
    #[cfg(not(feature = "std"))]
    pub trait UnboundedReentryRequiresTheDecycleStdFeature {}

    #[cfg(not(feature = "std"))]
    mod imp {
        use super::UnboundedReentryRequiresTheDecycleStdFeature;

        pub struct Registration;

        /// Ungated: opening a scope is harmless, and `register` below already carries the error.
        /// Reporting it once, at the registration, beats repeating it per frame.
        pub fn scope() -> Registration {
            Registration
        }

        pub fn register<K: ?Sized + UnboundedReentryRequiresTheDecycleStdFeature>(
            _fp: u64,
            _f: *const (),
        ) {
        }

        pub fn lookup<K: ?Sized + UnboundedReentryRequiresTheDecycleStdFeature>(
            _fp: u64,
        ) -> *const () {
            core::ptr::null()
        }
    }

    pub use imp::{lookup, register, scope, Registration};

}

#[doc(hidden)]
#[cfg(feature = "api")]
pub use decycle_impl::proc_macro_error;
