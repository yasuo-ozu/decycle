# Changelog

Notable changes, following [Keep a Changelog](https://keepachangelog.com/) and
[Semantic Versioning](https://semver.org/). Full defect analysis and design notes
for 0.4.0 live in `docs/unbounded-reentry-plan.md` (repository only).

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
- `mut`-pattern method params, GAT delegation, and renamed-crate detection (real
  TOML parse) fixed; `super::super::` paths, trait aliases, and `Fn(...)`-sugar
  bounds now get clean compile errors instead of misbehavior or panics.
- `allowed_paths` on a `#[decycle]` module is now a clean compile error (previously
  silently ignored); the associated-constraint bound diagnostic now states what is
  accepted (`Self` or the impl's own type parameters) instead of an inaccurate
  "non-local type" claim.

### Packaging

- `decycle` / `decycle-macro` / `decycle-impl` versioned in lockstep at 0.4.0;
  `rust-version = "1.87"` (empirical MSRV via `type-leak` → `gotgraph`); `docs/`
  excluded from the published crate.

[0.4.0]: https://github.com/yasuo-ozu/decycle/releases/tag/v0.4.0
