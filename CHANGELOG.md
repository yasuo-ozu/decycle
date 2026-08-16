# Changelog

Notable changes, following [Keep a Changelog](https://keepachangelog.com/) and
[Semantic Versioning](https://semver.org/).

## [Unreleased]

> **Release status (2026-08).** 0.5.0 is on crates.io and is what `cargo add decycle`
> resolves to; no release has been yanked. Everything in this section ships with 0.5.1
> (the version the workspace now carries).

### Fixed — soundness (2026-08-04 audit)

Three memory-unsafety classes, each pinned by `tests/ub_regressions.rs` and now
exercised by a miri CI job. These fixes close the specific classes the audit found;
they are not a claim that the engine as a whole is proven sound.

- **Ranked re-entry registry could call the wrong function (critical).** Registration was
  last-writer-wins, so a nested descent started from the middle of a method body (e.g. a
  visitor hook) overwrote the registry slot the enclosing frame would read at its floor;
  the floor then transmuted and called the *other* instantiation's fn — observably wrong
  results, and an observed SIGSEGV when the colliding closures' captures differed in kind —
  from entirely safe code at the default `recurse_level`. Registrations are now scoped: a
  per-frame guard rolls back what its frame's registrations *displaced* when the frame
  exits, including on unwind. A registration that created its slot deliberately persists —
  the documented "unbounded once primed" bare-param behavior and post-panic healing depend
  on that.
- **Fn-pointer provenance UB (ranked, every floor crossing).** The re-entry fn pointer was
  laundered through a `usize` (registered `as usize`, stored as `usize`, transmuted back
  and called), which strips provenance — UB on every unbounded floor crossing (miri:
  "it has no provenance"). The registry now stores `*const ()` end to end.
- **`&dyn Fn(&Self)` wrong-vtable UB (structural).** Forwarding a `&dyn Fn(&Self)`
  argument into terminator space transmuted the trait object, but `dyn Fn(&A)` and
  `dyn Fn(&__ATerm)` are *different traits*, so the wide pointer kept a vtable naming the
  wrong one (miri: "wrong trait in wide pointer vtable"); the size guard structurally
  cannot catch it. The argument is now re-wrapped in a stack adapter closure so the new
  trait object is built by ordinary compiler coercion. Other `Self`-mentioning
  higher-order shapes that cannot be rebuilt this way (`FnMut`/`FnOnce`, `&mut dyn ..`,
  by-value/boxed objects, extra bounds like `+ Send`, user traits) are now rejected with a
  compile error instead of being silently punned; the plain `fn(&Self)` pointer cast is
  ABI-compatible and unchanged.
- A decycled call running from a thread-local destructor now hits the ordinary fail-closed
  panic instead of aborting the process inside `std` with an `AccessError`.

### Changed — closures can no longer instantiate a cyclic method in unbounded mode (**breaking**)

- The re-entry registry keys on `type_name`, and rustc renders every closure and `async`
  block in a function as `{{closure}}`, with no disambiguator — two of them in one
  function produce the *same* key, and the layout fingerprint only separates closures
  whose captures differ in size/align (a property of the captures, not the code). A cyclic
  method instantiated with such an **anonymous type** is now rejected at runtime,
  deterministically on its first call — a panic whose message contains "anonymous type" —
  rather than depending on whether a collision actually occurs. Workarounds: pass a named
  function, coerce a non-capturing closure to a function pointer (`f as fn(_) -> _`), or
  set `support_infinite_cycle = false` (bounded mode emits no registry and accepts
  closures unchanged). The structural engine is unaffected.

### Changed — deep unbounded recursion uses ~12% more stack per frame

- The registration-scope guard must live across each frame's recursive call, so a deep
  unbounded descent costs more stack: 20,000 frames needed ~2.0 MiB before, ~2.25 MiB
  now — which straddles the 2 MiB default of a Rust test thread. Size the thread
  explicitly for very deep recursion.

### Added — public obligation-graph API: `decycle::analysis` (**breaking** re-export changes)

- New engine-independent `decycle::analysis` module — code-free inspection of a module's
  obligation graph over type idents, each edge labelled `Direct` or `Peeled`:
  `analyze_module` reads a module's classification back out, `cyclic_subgraph` restricts a
  graph to the nodes on a cycle, `with_nodes` adds participants discovered later. The
  result feeds `ranked::process_module_with_graph` / `structural::process_module_with_graph`.
  `decycle::safegraph` is re-exported so callers can name the returned `VecGraph` without
  depending on `safegraph` themselves.
- **Breaking:** the 0.3.0-era flat re-exports `decycle::process_module`,
  `decycle::process_trait`, and `decycle::finalize` are removed. The programmatic API now
  lives under the per-engine modules: `decycle::ranked` (`process_module`,
  `process_module_with_graph`, `process_trait`, `finalize::finalize`) and
  `decycle::structural` (`process_module`, `process_module_with_graph`).

### Added — reject value-level instantiation growth

A cyclic trait method that takes a generic parameter **by value** and **reborrows it** at the recursive
call now gets a clean, actionable error instead of a diagnostic naming generated internals:

```rust
#[decycle] pub trait Eval { fn eval<S: Src>(src: S, depth: u32) -> u32; }
impl Eval for A where B: Eval {
    fn eval<S: Src>(mut src: S, depth: u32) -> u32 {
        <B as Eval>::eval(&mut src, depth - 1)   // ← eval::<S> → eval::<&mut S> → …
    }
}
```

The *obligation* cycle is breakable and both engines break it; the **instantiation** cycle is not — the
growth is in the monomorphisation, one `&mut` layer per level. This is the value-level twin of the
non-regular where-bound already rejected in `ranked::finalize`, and like that one it cannot reject a
working program: rustc itself reports the class as `reached the recursion limit while instantiating`
(no error code, span only on the call), as an `E0275` against whatever blanket impl `&mut _` happens to
need — pointing at that innocent blanket — or, when only some call sites reborrow, as an ICE
(`failed to resolve instance for …`). None of the three names decycle, the growing parameter, or the
fix.

The check is engine-independent (both entry points run it), syntactic, and deliberately narrow, since a
false positive would reject a working program. It fires only when the impl's self type is a cycle
member, the method has a by-value method-level generic (including argument-position `impl Trait`), the
body calls a method of the *same name*, and the argument at that position is `&`/`&mut` rooted at a
binding derived from that parameter. So `eval(src, ..)` (moved on unchanged), `eval(&mut local_buf, ..)`
(a fixed type) and `eval_by_ref(&mut src, ..)` (callee takes `&mut S`) all still compile.

Taint follows `let` chains through method calls, because `let s = src.into_src();` is the real-world
spelling. A local with an **explicit type annotation naming no method generic** is treated as fixed, so
the one false positive the heuristic can produce is fixable in one line — and the diagnostic says so.

Pinned by `tests/ui/value_generic_growth_{ranked,structural,apit}.rs` and
`tests/ui/pass/value_generic_no_growth.rs`.

### Fixed — correctness and diagnostics

- **Structural: a method signature may project through a param bound.** The local
  `__DecycleBody` trait's declaration renders its generic params with every bound stripped
  (a bound may itself be the cyclic one, and re-stating it would put the cycle back). But
  `__run`'s signature is copied verbatim from the user's method, so a return or argument
  type of the form `<A as HasOut>::Out` made the declaration ill-formed on its own —
  `E0277: the trait bound A: HasOut is not satisfied` — even though every generated impl and
  the single call site could prove it. The declaration now re-states the **reduced** impl's
  where-predicates: cycle-free by construction, and provable at the call site, which sits
  inside the terminator impl carrying exactly them. Predicates whose *bounds* mention `Self`
  are skipped, since `Self` denotes the terminator there but the natural type at the
  declaration. Pinned by `tests/structural_body_trait_where.rs`.
- Premise sharing is scoped to the impls a rank chain can actually reach: an acyclic impl
  of a cycle's trait no longer silently inherits the cycle's side-bounds, and a predicate
  naming a sibling impl's lifetime is no longer injected into an impl without that
  lifetime (was E0261 on code that compiles without the macro).
- Cycle-head aliasing is scope-aware: an impl/method generic parameter that shadows a
  cycle-head name is no longer rewritten into the struct (was E0207 plus phantom bounds);
  cycle-head names inside an allowlist of std macros (`matches!`, `assert_eq!`, …) are now
  rewritten, so they no longer hit E0659 where a plain `match` compiled.
- A foreign type sharing only its last path segment with a cycle head is no longer
  misread as a cycle member (was a 14-error rank-lowering cascade against the foreign
  type; now a single error on the offending bound line).
- Ranked cross-edge calls on traits with lifetime parameters: the rank argument is now
  placed after the lifetime arguments actually *written* (elided lifetimes used to cause a
  bare proc-macro panic, and a partially-written argument list could put a user type in
  the rank slot).
- Structural: parameters/returns typed through a `Self` projection (`Self::Out`) are
  resolved against the impl's own associated types before casting (was E0308); `mut self`
  receivers now compile — and mutate.
- Two token-identical `#[decycle]` traits in one crate no longer collide (was E0428): the
  carrier-macro discriminant is now fresh per invocation instead of a hash of the trait's
  tokens.
- `decycle::analysis` no longer fabricates a graph edge from a bound targeting one of the
  impl's own generic parameters, and no longer panics on raw identifiers (`r#loop`).
- Front-door errors: `structural` on a *trait* item, a duplicated attribute argument
  (`recurse_level = 5, recurse_level = 2`), and a doubly-applied `#[decycle]` are now
  single clear errors instead of silent misbehavior or a diagnostic cascade.

### Added — structural unroll mode (`#[decycle(structural)]`)

- A second, self-contained algorithm for breaking method-recursion cycles, selected with the new
  `structural` flag: `#[decycle(structural)] mod … { … }`. It has **no runtime** (no registry, no
  re-entry fn pointers, no `type_name` keys) and its **generated code uses no `type-leak`** —
  everything is resolved at compile time. (The proc-macro crate always links `type-leak` — a
  non-optional dependency backing the trait-level `#[decycle]` attribute — so it is a build
  dependency regardless of which engine you use.)
- Mechanism: per cycle-member type it emits a `#[repr(transparent)]` terminator `__XxxTerm(pub Xxx)`
  (matching `Xxx`'s visibility) and puts each trait impl on the terminator via a *trait-def-inside-body*
  pattern (a private local trait whose method holds the original body verbatim, implemented for the
  natural type so `self`/`Self`/constructors resolve unchanged); the natural type's impl delegates to
  the terminator by a same-layout `transmute_copy`. The obligation cycle is broken by stripping the
  cyclic `where`-bounds; a bare cyclic bound is kept on the local body impl so it still pins otherwise-
  uninferable generics (`B: Frob<N>` for `B.frob(n)`), while resolving on sight through the natural impl.
- Scope: like the ranked engine it only unrolls impls of traits annotated `#[decycle]` in the module.
  Supports self-cycles, multi-type and multiroot cycles, **cross-trait** cycles (`Expr: Eval → Expr:
  Size → Expr: Eval`, keyed on a `(type, trait)`-pair obligation graph), all receiver shapes
  (`&self`/`&mut self`/owned/`self: Box<Self>`), method generics, argument-position `impl Trait`,
  associated types/consts, and multiple `#[decycle]` traits per type. When a *wrapped* cyclic bound is
  stripped (`Box<Stmt>: Tr`) it emits a compile-time forwarding assertion `for<T: Tr> Box<T>: Tr`.
- Not supported (use the default ranked engine): **growing-type-argument** recursion — a return-position
  `impl Trait` or a backtracking `Dup<…>`-style stream whose type grows one wrapper per descent level.
  `recurse_level` / `support_infinite_cycle` are rejected with `structural` (there is no rank floor —
  depth is bounded only by the OS call stack). Unbounded by construction.

### Added — `impl Trait` arguments bounded by a cyclic trait

- Input-position `impl Trait` in `#[decycle]` trait methods now works when the bound names
  a *cyclic* `#[decycle]` trait defined in the same module (`fn sink(&self, other: impl Feed, ..)`
  with `Feed` decycled). Previously this failed to compile (E0276 + E0277): the desugared
  method-generic bound was rank-lowered to `FeedRanked<Rank>` in the inductive impls but kept
  public on the ranked trait definition, and neither spelling is satisfiable for an argument
  that is a value from outside the cycle (threaded through every rank, and only ever `T: Feed`
  at the public boundary). The bound is now kept on the **public** trait consistently across the
  ranked trait definition, the inductive impls, the leaf impls, and the re-entry fn — the
  inductive impls qualify it with `super::` to escape the `shadowing_module` dummy that rebinds
  the bare trait name. Works in both modes and past the floor; the cyclic trait may be generic
  (`impl Feed<u8>`). Non-cyclic APIT bounds (`impl Fn(..)`, HRTB, multiple params) are unchanged
  at compile time — but note the separate Unreleased change above: instantiating a cyclic method
  with a *closure* (as opposed to a named type or fn pointer) is now rejected at runtime in
  unbounded mode. Return-position `impl Trait` remains a clean compile error in unbounded mode.

### Changed — return-position `impl Trait` (RPITIT) rejected earlier and more clearly

- A `#[decycle]` trait method with a return-position `impl Trait`, under the default
  `support_infinite_cycle = true`, is now rejected **up-front during module processing** with an
  actionable error that names the method and trait, instead of an `abort!` fired deep in re-entry
  codegen (which could leave a raw solver error alongside it). RPITIT is a genuine limitation of the
  ranked re-entry engine: full-height re-entry needs a nameable `fn(..) -> T` fn-pointer alias, and
  `fn(..) -> impl Trait` is `E0562`. Return a concrete or boxed type (e.g. `Box<dyn Trait>`), or set
  `support_infinite_cycle = false`. **Bounded mode is unchanged** — it builds no re-entry, so RPITIT
  there stays a plain rustc property (a diverging rank floor infers the hidden type as `()`).

## [0.4.0] — never published

> This version number was staged in-tree but **never released**: no `v0.4.0` git tag
> exists and crates.io went straight from 0.3.0 to 0.5.0. The changes below shipped as
> part of 0.5.0.

### Advisory

**Every release ≤ 0.3.0 is unsound at default settings**: with
`support_infinite_cycle = true` (the default), any recursion deeper than
`recurse_level` jumps through an incorrectly-transmuted pointer and crashes
(SIGSEGV). None of them has been yanked, so if you pin one, set
`support_infinite_cycle = false`. 0.5.0 and later do not have this defect.

### Changed — unbounded shim replaced

- New thread-local, fingerprinted `type_name`-keyed registry; the floor re-enters
  the original impl at **full height** through a generated re-entry fn. The only
  depth ceiling is the OS call stack.
- Registration is idempotent and register-before-descend: `recurse_level = 1` now
  works for any cycle width (was: needed `width + 1`, and still crashed past it).
- Generic methods are keyed **per instantiation** (incl. `impl Trait` args and
  phantom generics, which previously didn't even compile — E0283).
- `unsafe fn` / `extern "C" fn` methods, elided ref returns, unsized targets
  (`impl Ca for str`), and `?Sized` params now work in unbounded mode.
- Methods returning `Self::Assoc`/GATs now compile and can be driven past the
  floor from outside the cycle; consuming such projections *inside* cycle
  bodies remains unsupported.
- Residual unregisterable floors **fail closed** with an actionable, isolated panic
  (never memory unsafety): a generic method's first descent past the floor, bare
  type-param cyclic bounds (`impl<T: Cb> Ca for Wrap<T>`), and heterogeneous
  side-bound cycles. *Correction:* this entry originally claimed the backstop also kept
  *closure-keyed* floors sound. The 2026-08-04 audit disproved that — the backstop only
  checks that a key is present, not that it is correct, so a nested descent (or an entry
  left behind by a panicked one) could make the floor call the wrong closure's fn, up to a
  SIGSEGV from safe code. Closure instantiation is now rejected outright; see the
  Unreleased section.
- `recurse_level = 0` is a clean compile error. The previously-disabled 6-trait
  dense-cycle test now passes in both modes. `support_infinite_cycle = false`
  is unchanged (zero-cost, `unimplemented!` at the limit).

### Fixed — pre-existing ranked-trait machinery

- `#[decycle] use path::T as R;` no longer silently drops the renamed trait's impls,
  and cross-edge calls to the renamed trait's methods resolve correctly (was E0034).
- Trait-level `const` generic parameters on a `#[decycle]` trait are now supported
  (previously E0747 in generated code; method-level const generics already worked).
- Non-cyclic multi-segment `where`-bounds (e.g. `Self: ::core::fmt::Debug`) are no
  longer stripped; defaulted trait methods no longer cause E0046.
- `self::`-qualified and two-segment `Trait::method` references are rewritten like
  bare names; same-named `#[decycle]` traits in one crate no longer collide (E0428).
- `mut`-pattern method params and GAT delegation fixed; renamed-crate detection now
  matches the name passed as `#[decycle(decycle = ::my_rename)]` (it no longer reads the
  consumer's `Cargo.toml`); `super::super::` paths, trait aliases, and `Fn(...)`-sugar
  bounds now get clean compile errors instead of misbehavior or panics.
- `allowed_paths` on a `#[decycle]` module is now a clean compile error (previously
  silently ignored); the associated-constraint bound diagnostic now states what is
  accepted (`Self` or the impl's own type parameters) instead of an inaccurate
  "non-local type" claim.

### Packaging

- `decycle` / `decycle-macro` / `decycle-impl` versioned in lockstep at 0.4.0;
  `rust-version = "1.71"` — the floor for any `syn 2.0`-based proc-macro crate, set by the
  latest `syn` / `quote` / `unicode-ident`. Down from a would-be 1.87 by switching to
  `type-leak 0.7` (backed by `safegraph`, MSRV 1.56) from `type-leak 0.6` (backed by
  `gotgraph`, empirical MSRV 1.87), and by dropping the `toml` dependency (renamed-crate
  detection no longer parses the consumer's `Cargo.toml`). `docs/` excluded from the
  published crate.
