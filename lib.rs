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
/// Engine-independent, code-free inspection: the module's obligation graph over type idents, with
/// each edge labelled `Direct` or `Peeled`.
///
/// The analysis itself is the same whichever engine you go on to use — it lives here, and only here.
/// What differs per engine is how a graph is *consumed*: [`ranked::process_module_with_graph`]
/// **replaces** its participant set with the graph's nodes, whereas
/// [`structural::process_module_with_graph`] can only **filter** with them, its model being keyed on
/// `(type, trait)` pairs rather than on types.
/// Engine-independent, code-free inspection: the module's obligation graph over type idents, with
/// each edge labelled `Direct` or `Peeled`.
///
/// - [`analysis::analyze_module`] reads a module's own classification back out;
/// - [`analysis::cyclic_subgraph`] restricts a graph to the nodes that lie on a cycle, so a caller
///   can supply a whole reference relation and let decycle decide what recurses;
/// - [`analysis::with_nodes`] adds participants discovered after the graph was built.
///
/// The result is what [`ranked::process_module_with_graph`] and
/// [`structural::process_module_with_graph`] take.
pub use decycle_impl::analysis;
/// Re-exported so callers can name the [`safegraph::VecGraph`] that
/// [`analysis::analyze_module`] returns without depending on `safegraph` themselves.
pub use decycle_impl::safegraph;
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
/// compile time; ranked can re-enter across several instantiations of a generic method, but a residual
/// set of shapes fails closed at runtime). See the crate README's *Two algorithms* section for the full
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
/// implemented for your own types. For the complete per-engine matrix — multi-instantiation re-entry,
/// `#[track_caller]`, `no_std`, higher-order `Self`-mentioning arguments
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
        let found = REG
            .try_with(|reg| reg.borrow().get(&(type_name::<K>(), fp)).copied())
            .ok()
            .flatten();
        found.expect(
            "decycle: re-entry fn not registered before the floor was reached. This floor's \
             key had no same-instantiation frame run on this thread's descent first — e.g. a \
             generic method's first descent at cycle width > recurse_level (including \
             self-recursion consuming ranks before the first generic cross-edge call), or an \
             impl whose cyclic bound targets a bare type parameter. Increase recurse_level. \
             (This can also fire when the call runs from a thread-local destructor, after the \
             registry for this thread has already been torn down.)",
        )
    }
}

#[doc(hidden)]
pub use decycle_impl::proc_macro_error;
#[doc(hidden)]
pub use decycle_impl::type_leak;
