# Changelog

Notable changes, following [Keep a Changelog](https://keepachangelog.com/) and
[Semantic Versioning](https://semver.org/). Full defect analysis and design notes
for 0.4.0 live in `docs/unbounded-reentry-plan.md` (repository only).

## [Unreleased]

### Added — structural unroll mode (`#[decycle(structural)]`)

- A second, self-contained algorithm for breaking method-recursion cycles, selected with the new
  `structural` flag: `#[decycle(structural)] mod … { … }`. It has **no runtime** (no registry, no
  re-entry fn pointers, no `type_name` keys) and **no `type-leak` dependency** — everything is
  resolved at compile time.
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
  (`impl Feed<u8>`). Non-cyclic APIT bounds (`impl Fn(..)`, HRTB, multiple params) are unchanged;
  return-position `impl Trait` remains a clean compile error in unbounded mode.

## [0.4.0]

### Advisory

**All previous releases (≤ 0.3.0) are unsound at default settings and should be
yanked/avoided**: with `support_infinite_cycle = true` (the default), any recursion
deeper than `recurse_level` jumps through an incorrectly-transmuted pointer and
crashes (SIGSEGV). Workaround on old versions: `support_infinite_cycle = false`.

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
  side-bound cycles.
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

[0.4.0]: https://github.com/yasuo-ozu/decycle/releases/tag/v0.4.0
