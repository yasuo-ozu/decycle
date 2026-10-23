# Changelog

Notable changes, following [Keep a Changelog](https://keepachangelog.com/) and
[Semantic Versioning](https://semver.org/).

## [Unreleased]

The changes below break existing code (see **Changed** and **Removed**), so they are to be released
as **0.8.0**, not as a 0.7 patch.

### Added

- **`module = path` on a `#[decycle]` module** (`#[decycle::decycle(module = crate::m)] mod m { .. }`,
  combinable with `structural`, `recurse_level`, ..): the module's own absolute path. It must start
  with `crate` (or `$crate`, as a `macro_rules!` passes it, or a leading `::`), carry no generic
  arguments and end in the module's name (a leading `::` needs `extern crate self as ..`); anything
  else is an error with a help line, reported after the module is expanded without it (an
  `#[announce]`d item in it then reports its relative paths as well). On a trait it is rejected like
  the other module options. The cycle is still solved on the module as written, where only a bare
  (or `self::`) name is the module's (a type or trait named by a path, `crate::m::Term: Eval`, stays
  the opt-out). Once it is solved:
  - the items emitted as written — structs, enums, unions, type aliases, traits, fns, consts,
    statics, and every impl the engine does not take over (inherent impls, impls of other traits,
    path-spelled opt-outs) — have each bare or `self::` name of a type or trait declared directly in
    the module spelled `path::X`, in type and bound positions (fields, the types of consts and
    statics, generics, where clauses, supertraits, signatures, impl headers, associated types). This
    happens before the attributes on those items expand, so a struct or enum inside the module can
    be `#[announce]`d (announce requires absolute paths) with bare fields such as `Box<Term>` or
    `Box<self::Term>`. Bare names shadowed by a generic parameter, `Self`, other multi-segment
    paths, fn bodies and other expressions, macro invocations, nested modules and imported names
    are left as written. The impls of the `#[decycle]` traits are generated anew by the engines and
    keep the bare spelling. The rewrite is syntactic: a type declared under an inactive `#[cfg]`
    still counts as declared (so its name is qualified), and a type a macro invocation declares
    does not.
  - the path is checked against the module: a hidden marker item is declared in the module and named
    through the path, so a path that is well-formed but not this module's (`crate::m` on
    `crate::outer::m`, which would otherwise quietly retarget a name to another module's item of
    the same name) fails with an error at the `module` argument (``cannot find type
    `__DecycleModule_..` in module `crate::m` ``), as does a path that names no module at all.

  Without `module` nothing changes, except for the spelling rules below. The programmatic forms are
  `ranked::process_module_at` and `structural::process_module_at`.

- **With `module = path`, the ranked engine reports an opt-out bound on a cycle itself**, in the
  module's terms, where rustc would report `E0275` on the engine's internals (``overflow evaluating
  the requirement `A: ranked_traits…::LoopRanked…<((((((_,),),),),),)>` ``). A bound that names one
  of the module's own structs, enums or unions by its path, `<module>::C` or a `super::m::C` that is
  the same path once `super` is resolved against the module path, directly
  (`where crate::m::C: Loop`) or as a part a peeled wrapper keeps (`(B, crate::m::C): Loop`), is
  checked as if it were bare. When `C`'s impl leads back to the impl carrying the bound, the error
  is at the bound, with the cycle:

  ```text
  error: decycle: `crate::m::C` names `C` of this module by its path, so this bound is left as written, but `C`'s impl of `Loop` leads back to this impl (`A` → `C` → `A`): rustc cannot prove that cycle (E0275)
           = help: write `C` to make the bound part of the cycle decycle breaks
  ```

  Every such bound of the module is reported, and nothing else: the module emitted after the errors
  drops the bounds of the `#[decycle]` traits that name no generic parameter, which rustc would
  otherwise report as overflowing on their own. Only a certain overflow is reported, and only one
  that the module states: the chain must run through impls without generic parameters, for types
  declared in the module and named without arguments, each the only impl of the trait for its type
  and carrying no attribute but lint levels and docs (no `#[cfg]`, no attribute macro), along
  bounds that name the same trait bare. Anything else on the way (a generic impl such as
  `impl<T> Loop for Wrap<T> where T: Loop`, whose answer depends on `T`, a type with arguments,
  another trait, an associated-type constraint, a `for<..>` binder), a module under an attribute
  that is not inert, and every module without `module` are left to rustc as before. The rank chain
  is taken into account: a chain back with more bare bounds than `recurse_level` reaches the rank
  floor first and compiles, so it is not reported (a call that reaches the floor on it fails
  closed at run time, "re-entry fn not registered"). The structural engine is unchanged: its
  `E0275` already names the module's types (`A: m::Loop`), and it does not peel a wrapped bound
  into parts.

- **A `#[decycle]` trait may have another `#[decycle]` trait of the module as a supertrait**
  (`trait Eval: Base`, or `self::Base`) with the ranked engine, which used to reject it ("a
  #[decycle] trait cannot be a supertrait of another #[decycle] trait"; a path-spelled supertrait
  slipped past the check). The ranked twin of the trait takes the supertrait's ranked twin at the
  same rank (`EvalRanked<Rank>: BaseRanked<Rank>`), so a subtrait impl's body can call the
  supertrait's methods on `Self` and on cycle members at any depth, chains (`A: B`, `B: C`) and
  several supertraits (`Eval: Base + Other`) included, and generic impls (`impl<T> Eval for Wrap<T>
  where T: Eval`) get the `T: Base` they need from `T: Eval`. The supertrait's re-entry fns are
  registered for `Self` in the subtrait's frames, so a recursion past the floor that enters the
  supertrait's chain finds them. A supertrait implemented outside the module for some type of the
  subtrait, and one named by a path (`crate::m::Base`, the opt-out), stay the real trait as written:
  the impls work, but a subtrait impl's body can call that supertrait's methods only through its
  path (by method syntax the call does not compile: `E0283`/`E0275` on `BaseRanked…`). One case
  still fails closed past the
  floor with the documented "re-entry fn not registered" panic: a generic supertrait method called
  from a subtrait impl's body on a type whose supertrait impl is generic over its cyclic parameter
  (`impl<T: Base> Base for Wrap<T>`), as any generic method first reached from a non-generic
  caller does. The structural engine is unchanged: its terminators implement only the pairs of
  their own cycle, so a supertrait outside the cycle (acyclic, implemented elsewhere, or reached
  through a cross-trait bound only) is `E0277` on the terminator.

### Changed

- **Breaking: both engines read a cycle participant only by the name the module gives it**, bare
  or `self::`-qualified: the trait of an impl, the trait and the types of a bound, and the self type
  of an impl. Anything the module writes as another path (`crate::m::X`, `super::X`,
  `crate::other::X`, `::krate::m::X`) names the item itself, the opt-out: it is left as written, so
  a cycle through it fails to compile, not at run time (but for a ranked chain back longer than
  `recurse_level`, see **Added**). With the structural engine that is rustc's own overflow
  (`E0275`) on the impls as written; with the ranked engine it is `E0275` too, which can name the
  engine's `…Ranked` traits (and, in a module with several `#[decycle]` traits, an `E0277` on
  internal names), unless `module = path` lets decycle report it first (see **Added**). In detail:
  - **structural**: the trait of an impl (`impl crate::m::Eval for X`, `impl super::Eval for X`),
    the trait and the types of a bound (`where Y: super::Eval`, `where crate::m::Term: Eval`) and
    the self type of an impl (`impl Eval for crate::m::Term`) used to be matched by their last
    segment. They are now the opt-out and are emitted as written; `impl Loop for crate::other::B`
    beside a cycle member `B` is no longer `E0119`, and a bound on a foreign wrapper sharing a
    member's name (`where crate::other::B<B>: Loop`) is a wrapper around the member (forwarded,
    with its assertion), no longer `E0107`. Reading bound types through `syn`'s visitor also
    changes two classifications: a member inside an associated-type binding is seen
    (`P<dyn Kind<Out = Term>>: Eval` is now forwarded as a wrapped bound), and a projection is no
    longer misread as the ADT that shares its name (`<U as Tr>::Term` is not `Term`).
  - **ranked**: an impl whose self type is spelled by a path (`impl Eval for crate::m::Term`) is no
    longer taken over — it used to be, and then missed by the rest of the engine (an `E0275` wall
    naming `EvalRanked…`). It is the opt-out now, emitted as written.
  - **`self::`**: a `self::` self type (`impl Eval for self::Term`) or bound type
    (`where self::Term: Eval`) is the bare member in both engines. The ranked engine used to drop it
    out of the cycle (an `E0275` wall naming `EvalRanked…`).
  - **ranked, wrapped bounds**: a cyclic bound on a wrapper is replaced by the obligations of all
    its parts, on the contract that a wrapper implements the trait when its type arguments do. A
    cycle member is an edge, as before, and a part that contains no member is now kept as a premise
    on the original trait, as written: `(B, crate::m::C): Loop` is `B: Loop` and
    `crate::m::C: Loop`, `(B, u8): Loop` is `B: Loop` and `u8: Loop`. Those parts used to be
    dropped, so a path-spelled cycle type inside a wrapper (`(B, crate::m::C)`, `Vec<(B,
    crate::m::C)>`, `Box<(B, Box<crate::m::C>)>`) panicked at run time ("re-entry fn not registered
    before the floor was reached") where it now fails to compile, and a bound that cannot hold
    (`(B, u8): Loop` with no `u8` impl) was accepted where it is now `E0277`. The `for<..>` binder
    and the extra non-cyclic bounds of the predicate are kept as before.
- **The structural engine shares premises among the members of each cycle, always**, as the
  ranked engine does: each impl of a trait in a cycle gets the `where`-predicates of the cycle's
  other impls of that trait that are no edge of the cycle (its leaf premises). A member's generated
  impl reaches a sibling through the public trait, and the sibling's impl demands its own leaf
  premises, which the member need not state (`Stmt` reaching a `#[group]`-like `Group` whose impl
  alone needs `A: Clone`): that used to be `E0277` on the member, unless a graph was supplied
  (`process_module_with_graph`, which shared across every impl of the trait). The premises land on
  the member's generated impls, the natural type's impl included (`impl<T> Tr<T> for A where T:
  Clone`), which changes nothing it effectively requires: the stripped cyclic bound required them
  already. Impls outside every cycle, and the impls of another cycle, are left alone. The premises
  travel along the cycle's bounds, whatever their trait (see **Fixed**): an impl receives those of
  every impl its cyclic bounds reach, transitively, with the reached impl's generic parameters
  mapped through the bound, so `impl<U> Size<U> for Term where U: Clone` gives `impl<T> Eval<T> for
  Expr where Term: Size<T>` the bound `T: Clone`, and through `Term: Size<u8>` only `u8: Clone`.
  Inline parameter bounds and the non-cyclic co-bounds of a cyclic bound (`Clone` in
  `Term: Size<T> + Clone`) count as premises, and a premise naming `Self` is given with `Self`
  spelled as the reached impl's self type (`Self: Debug` on `Term<R>` reaches `Expr<S>` as
  `Term<S>: Debug`). A premise that cannot be said in the receiving impl's terms is skipped,
  without a diagnostic (if it was needed, the error is the `E0277` there would have been without
  it): one naming a lifetime of the reached impl that no argument of the bound fixes
  (`impl<'u, T> .. where T: 'u`), one naming an item the receiving impl shadows with a parameter of
  the same name, a projection `T::Out` whose `T` maps to a type that is no plain name, and one
  behind a macro. Beside that, the impls of one trait in a cycle still take the union of each
  other's premises by name, as in the ranked engine: a predicate is only given to an impl that
  declares every generic parameter it names, and a `Self`-mentioning one is not.
- **Breaking (ranked): a leading `::` on a one-segment trait path is a path, the opt-out**, as it
  already was in the structural engine and for types in both: `impl ::Tr for X`, `where Y: ::Tr`
  and `<Y as ::Tr>::f` name the item `::Tr` itself, not the module's `Tr`, and are left as written.
  The ranked engine read them as the bare `Tr` and rewrote them into the cycle, so a spelling that
  names nothing in edition 2018 on (`::Tr` is a crate there) compiled and ran; rustc now reports it
  (``E0405: cannot find crate `Tr` in the list of imported crates``). Write `Tr` (or `self::Tr`).
- **Breaking (structural): a `#[decycle(structural)]` module rejects three forms the ranked engine
  already rejects, with the same errors:**
  - a glob `#[decycle] use path::*;`, which compiled and routed no trait (`glob is not supported in
    #[decycle] use`; name the traits, `use path::{Tr, Other};`);
  - a `#[decycle]` anywhere inside a nested module, which compiled: a nested `#[decycle] mod` was
    expanded on its own, and a marked item of a plain nested module as if outside any `#[decycle]`
    module (`#[decycle] is not supported in nested modules`; move the module out of the
    `#[decycle]` module);
  - `#[decycle]` on a trait alias (`#[decycle] is not supported on a trait alias`). The marker used
    to be left on the alias, where it failed as an unknown attribute.

### Removed

- **Breaking: the obligation-graph API**, without a deprecation period: `ranked::
  process_module_with_graph`, `ranked::finalize::finalize_with_graph` (and their `emit_contracts`
  re-spelling), `structural::process_module_with_graph`, the `decycle::analysis` module
  (`analyze_module`, `analyze_items`, `cyclic_subgraph`, `with_nodes`, `EdgeKind`) and the
  `decycle::safegraph` re-export. The engines take their participants from the module's own
  spelling only. The plain programmatic entry points stay: `ranked::process_module`,
  `ranked::process_module_at`, `ranked::finalize::finalize` (with its `AlsoRank` hook) and
  `structural::process_module`/`process_module_at`, behind the `api` feature as before.

  **Migrating a macro that generates the impls it hands an engine** (the graph's use case): spell
  the module for the engine instead of stating its cycle — bring each routed trait in with
  `#[decycle] use path::to::Trait;` and call the plain `process_module`, with
  - every impl header naming the routed trait bare (`impl<S, A> Parse<A> for Expr<S>`);
  - a where-bound naming it bare only when its bounded type names a type the module declares,
    generated helper types (`#[group]` substructs) included: `Vec<Box<Stmt<S>>>: Parse<A>`;
  - every other bound naming it fully qualified (`crate::lexer::Integer: ::syan::Parse<A>`), which
    makes it a premise rather than an edge.

  The leaf premises the graph path used to share across a trait's impls are now shared among each
  cycle's members by both engines (see **Changed**).

### Fixed

- **The ranked engine takes every `#[decycle]` marker of an item, as the structural engine does.**
  An item marked twice (`#[decycle] #[decycle] trait Tr`, as a macro that marks the items it emits
  can leave one the user marked already) is marked once. The ranked engine took only the first
  marker and re-emitted the second, which then failed as an unknown attribute (``cannot find
  attribute `decycle` in this scope``; on a `use`, `place it inside module annotated with
  #[decycle]`), or, with `decycle` imported into the module, announced the trait as if it stood
  outside any `#[decycle]` module. Each marker is checked, the second one too: an argument list on
  it is the error a first one gets (`#[decycle] takes no arguments on an item inside a #[decycle]
  module`). A marker inside `#[cfg_attr(..)]` is still not read as one, by either engine.
- **A cycle that alternates between traits shares its premises**, in both engines (`Expr: Eval`
  needing `Term: Size`, which needs `Expr: Eval`; three traits or more alike). Premises were shared
  among the impls of one trait only, so `Expr`, the cycle's only `Eval` impl, never received
  `Term`'s `T: Clone`. With `structural` that was `E0277` on the generated impl (``the trait bound
  `T: Clone` is not satisfied``); the ranked engine compiled it, but skipped the re-entry
  registrations that need the premise, and a recursion past the floor panicked ("re-entry fn not
  registered before the floor was reached"). That was the documented heterogeneous side-bound
  limitation (`impl<T: Clone> Ca for A<T> where B<T>: Cb` beside `impl<T: Default> Cb for B<T>
  where A<T>: Ca`), which now runs unbounded. Each impl now receives the premises of the impls its
  cyclic bounds reach, whatever their trait, with the generic parameters mapped through the bound
  (not matched by name), as described for the structural engine under **Changed**. The ranked
  engine only carries a premise its registration check reads the same way, so it leaves out a
  premise naming `Self`, and a premise with a trait bound that names a lifetime other than
  `'static`, has a `for<..>` binder, names a const parameter or projects from a parameter
  (`T::Out`), or has a parameter in `Fn(..)` sugar or an associated-item constraint of the bound;
  a side bound of that kind still fails closed past the floor. A premise given to a parameter that
  has inline bounds joins them (`impl<T: Clone + Default>`), so the generated impl does not trip
  `clippy::multiple_bound_locations` on the user's impl header.
- **A floor reached by elimination is a compile error (`E0283`) instead of a run-time panic.** In
  a ranked module, a call of a `#[decycle]` trait's method whose rank nothing pins leaves the rank
  of the ranked twin to inference: one on a type the impl has no bound for (`self.base()` in an impl
  of another trait, with no `Self: Base`), or one through an impl of the module whose step impl
  cannot apply (a pair impl `impl<X: Loop, Y: Loop> Loop for (X, Y)` called as
  `(B, Foreign).step(n)`, with `Foreign`'s impl outside the module and no ranked impl forwarding to
  it: see the next entry for when it gets one). Where the step impl
  (`XxxRanked<(R,)>`) was ruled out, the floor (`XxxRanked<()>`) was the one impl left and was
  selected, though no frame had registered anything for it: the call compiled and panicked at run
  time, "re-entry fn not registered" (or "cycle limit reached" with
  `support_infinite_cycle = false`). Every solver rules the step impl out for the pair. For
  `self.base()`, the old solver (stable's) reported `E0283` already, but the next-generation
  solver, nightly's default since rust-lang/rust#160619 (2026-08-21) and planned for stabilisation,
  rules it out through the caller's where-bounds. Each floor now has a twin at an uninhabited rank
  no other rank unifies with, so two impls are left and every solver reports the call as ambiguous.
  A pinned rank is unaffected. On nightly, a call of the first shape that ran because an earlier
  frame had happened to register its callee now fails to compile as well. Pin the call with a bound
  (`Self: Base`), a supertrait (`trait Eval: Base`) or a call by path
  (`crate::m::Base::base(self)`); see the README for the pair.
  `tests/ui/ranked_floor_by_elimination*.rs` pin both shapes, blessed on the old solver; to check
  them on the next one, run
  `cargo +nightly test --test trybuild -- --ignored ui_compile_fail trybuild=ranked_floor_by_elimination`
  (it passes with the same snapshots).
- **Ranked engine: a call through a wrapper whose parts are partly outside the cycle compiles and
  runs.** `impl Loop for A where (B, crate::other::Leaf): Loop`, with the pair's impl in the module
  (`impl<X: Loop, Y: Loop> Loop for (X, Y)`) and `Leaf: Loop` implemented outside it, could not call
  through the pair: `(B, crate::other::Leaf).step(n)` panicked on the first call ("re-entry fn not
  registered before the floor was reached"), and with the floor's decoy twin (above) it is `E0283`;
  the same call by the trait path was `E0277` on `Leaf: LoopRanked…`. The bound was replaced by its
  parts, so nothing fixed the rank of the call, and the pair's ranked step demanded
  `Leaf: LoopRanked<R>`, which a type outside the cycle never had. The engine now keeps the
  outermost wrapper the module implements next to its parts, as an edge, registers the floors of
  the chain through it, and gives every type outside the cycle the chain reaches (here `Leaf`;
  `Vec<U>` for a parameter `U`) a ranked impl that forwards each method, associated type and const
  to the original trait — for a ranked supertrait too. The wrapper's floor (and its decoy) ranks its
  inline bounds (`X: LoopRanked<()>`) instead of demanding the original trait, which closed an
  `E0275` cycle through the members' delegating impls; with it, a cycle through a participating
  container (`Wrap<Stmt>: Tr` with `impl<T: Tr> Tr for Wrap<T>`, `tests/nested_participant_cycle.rs`)
  no longer overflows. No such impl is written where it could overlap another ranked impl: for a
  part named through a `use .. as` rename, a type alias or a relative path, which may be a cycle
  member under another name (such a name is now spelled `self::X` in the bounds of the impls taken
  over, which reads the same); for a part an impl of the module may implement
  (`::std::vec::Vec<u8>` beside `impl<T: Loop> Loop for Vec<T>`, which is planned through instead
  when spelled `Vec<u8>`), or a path to a member (`crate::m::C`); or for a part the module spells
  two ways (`Leaf` and `crate::other::Leaf`). There the bound reads as its parts, as before, and the
  call through the wrapper stays `E0283` (`tests/ui/ranked_floor_by_elimination_tuple.rs`, which
  pinned the call above and now pins a renamed part). A wrapper whose impl is outside the module is
  still reached by path only (`tests/ui/wrapper_outside_method.rs`).
- **The ranked engine's expansion is deterministic.** It emitted its per-trait items (the ranked
  twins and re-entry items, the dummy shadows, the inductive and the Final impls) in the iteration
  order of a `HashMap`, which `std` seeds per process: a module with several `#[decycle]` traits
  expanded differently from one compilation to the next (and between two expansions in one
  process). The same
  items, in another order; with several errors, which one an up-front check reported varied too.
  They now follow the traits' declaration order.
- The ranked engine no longer panics at run time ("re-entry fn not registered before the floor
  was reached") when a member of a cycle of several impls has a premise on the original trait — a
  bare bound on a type that is no participant (`crate::other::Stmt: Tr`, or a part the wrapped-bound
  peel now keeps) — and the recursion passes the floor. The reachability walk behind the re-entry
  registrations followed the premise as an edge, found no impl to descend into and gave up, and
  the premise was not shared with the member's siblings (it is now, like any other premise).

## [0.7.0] — 2026-10-18

### Changed

- **Breaking: announce 0.2.** `decycle`, `decycle-macro`, `decycle-impl` and the re-export
  `decycle::announce` take announce (and `announce-std`) 0.2.0. Every call of the trampoline chain now
  starts with `@announce_format "1"`, which a trampoline made by announce 0.1 does not read: a trait
  announced by announce 0.1 (or by decycle 0.6, which re-exports it) cannot be `#[decycle] use`d by
  this version and fails with announce's "invalid trampoline input"; the crate that defines it has to
  move to announce 0.2 too. `FinalizeArgs` accepts the prefix (`ContList::invoke` writes it), and a
  `macro_rules!` callback that parses what the trampolines call has to accept it:
  `(@announce_format "1" [$($list:tt)*] ..)`.
- Several announcements of one trait make one trampoline in announce 0.2, so `#[decycle]` on a trait,
  which expands to `#[announce]`, combines with an `#[announce(..)]` written beside it
  (`decycle::check`), in either order, where announce 0.1 made a second trampoline (E0252). The
  expansions of `#[decycle] mod` are unchanged. The MSRV stays 1.71 (announce 0.2 needs 1.57).

## [0.6.0] — 2026-10-15

### Changed

- **Breaking: `#[decycle]` on a trait outside a `#[decycle] mod` now just announces it.** It
  expands to [`#[announce]`](https://docs.rs/announce) (re-exported as `decycle::announce`):
  `#[decycle::announce::announce(crate = ::decycle::announce, allow_std_prelude = true)]`, or
  `crate = PATH::announce` with `decycle = PATH`. Write `#[announce]` directly for its options
  (`mock`, checks). The trait is carried by announce's own trampoline, as written, and
  brought into a module with `#[decycle] use` as before; the module-side machinery is unchanged. The
  other options of the old form are errors:

  | was | now |
  | --- | --- |
  | `#[decycle]` | `#[decycle]` (expands to `#[announce(crate = ::decycle::announce, allow_std_prelude = true)]`) |
  | `alter_macro_name = N` | none: `#[decycle] use` the trait by its own name |
  | `decycle = path` | unchanged: the trait is announced with `crate = path::announce` |
  | `marker = path` | none: write the trait's paths absolutely (`crate::..`, `::krate::..`) |
  
  Writing `alter_macro_name`, `marker` or `mock` is an error that names its replacement, and so is a
  module option (`structural`, `recurse_level`, `support_infinite_cycle`) on a trait.
- **`type-leak` is no longer a dependency** of `decycle`, `decycle-macro` or `decycle-impl` (it
  remains in the graph only as a dependency of `announce`). The relative-type interning it did for
  `#[decycle] trait` (`marker`, `allowed_paths`, the hidden `Repeater` trait) is gone with that form:
  `#[announce]` checks at the definition that every path in the trait resolves in other crates, and
  reports a relative one (`Local`, `super::X`, `std::vec::Vec<u8>`) as an error naming it. A trait
  that used `marker` to carry a relative type spells it absolutely now (`crate::Local`); bare std
  prelude names (`Clone`, `Vec<u8>`, `Option<T>`) stay accepted by `#[decycle]` (it passes
  `allow_std_prelude = true`), but a plain `#[announce]` needs that option. announce's own
  `repeater` option is not supported by `#[decycle] use`. `allowed_paths` is no longer recognized
  at all (an unknown option).
- Removed from the public surface: `decycle::type_leak`, `decycle_impl::type_leak`,
  `decycle_impl::process_trait` (nothing remains for it to do) and the hidden
  `decycle::Repeater`.
- **New default feature `announce`.** It gates the `decycle::announce` re-export; everything else
  (`#[decycle] mod`, `#[decycle] use`, `decycle::check`) needs no feature. It is taken from announce
  0.1.1 without that crate's `proc-macro-api` feature (`proc-macro2`/`syn`, `announce::helper`), so it
  does build for a core-only target: a `#![no_std]` crate with
  `decycle = { default-features = false, features = ["announce"] }` can announce a trait
  (`#[announce]`, `decycle::check`) and use `#[decycle]` (CI builds such a consumer for
  `x86_64-unknown-none`). `api` is what stays off for `no_std`, as before.
- The trampoline's wire format changed: `FinalizeArgs` now parses an announce input
  `[list] ENTRY* REST`, and its `ToTokens` writes `[list] REST`; `FinalizeArgs::cont_list` and
  `FinalizeArgs::rest_tokens` are new. A trait defined with decycle 0.5 is not announced and cannot
  be `#[decycle] use`d from 0.6. An announced trait depends on announce 0.1 and not on decycle at all,
  so the crate that defines it does not have to run the same decycle version as the one that uses it.
- **`decycle-impl` uses `safegraph` 0.3** (was 0.2), the version `type-leak` 0.8 uses through
  announce, so one copy of safegraph builds instead of two. `decycle::safegraph` (the re-export) and
  the `VecGraph` that `analysis::analyze_module` returns follow: in 0.3 a scope hands out
  `NodeIxRef`/`EdgeIxRef` handles (`ctx.node_indices()`, `ctx.edge_indices()`), turned into the
  copyable `NodeIx`/`EdgeIx` with `.to_ix()`, and `ctx.endpoints(e)` is an iterator rather than an
  array.
- **A stand-in named differently from the trait it stands in for has to be renamed**
  (`mock = path` with a name of its own: `#[decycle] use stand::Sz as Size;`). A stand-in is not the
  trait, so left unrenamed the impls would not be matched to it and the module would fail with an
  unrelated `E0275`; the ranked engine reports an error that names the `as` to write. (The structural
  engine reads no trait through a `#[decycle] use`, so it cannot tell, and still fails with the
  `E0275`.) The alias is the name the impls spell, so a stand-in `use`d `as` a third name
  (`use real::Size as Cnt; #[decycle] use stand::Sz as Cnt;`) works too. Under
  `#[decycle(structural)]`, which reads no trait through a `#[decycle] use`, the `use` is exempt from
  `unused_imports`, so such a renamed stand-in does not fail `#![deny(warnings)]`.

### Added

- `decycle::announce`, the re-export of the announce crate (feature `announce`), and
  `decycle::check`: list it among the options of `#[announce]` to validate a trait at its
  definition. It rejects a default on a generic parameter, a return-position `impl Trait` in a
  method and an `async fn` in a method, the rules that depend on the trait alone, with the error a
  `#[decycle]` module would give when it receives the trait. The default-parameter rule belongs to
  the ranked engine only, and `decycle::check` enforces the ranked engine's rules;
  `decycle::check(structural)` skips it for a trait meant for `#[decycle(structural)]`, which accepts
  a default. (The functions behind them in `decycle-impl` are internal, `#[doc(hidden)]`, and not an
  API.)
- A plain `#[announce]` trait can be brought in with `#[decycle] use`, including an
  `#[announce(mock = ..)]` copy such as `announce_std::fmt::Display` (treated as the real trait).
  `#[announce(mock = path)]` declares a stand-in for the real trait at `path`: only its trampoline
  `Stand!` is emitted, at the item's path and visibility, and `#[decycle] use` sites treat the entry
  as the real trait (its ident becomes `path`'s last segment, so `impl RealTrait for X` is matched).
- `#[decycle] use` of an announced item that is not a trait is a clear error at the `use`
  (`` #[decycle] use: `X!` is not a trait ``), not at the definition, which may be in another crate.

### Fixed

- Nested `#[cfg]`s in a foreign announced trait are evaluated in the defining crate, so a
  cfg'd-out method no longer reaches the `#[decycle] use` site. In 0.5.3 the consuming crate
  evaluated them, and a method gated with `#[cfg(any())]` failed with `E0576` (cannot find method
  `gone` in trait `Gen`).
- `$crate` and `$(..)*` macros in default method bodies of a foreign announced trait work.

## [0.5.3] — 2026-09-27

### Changed

- **A `macro_rules!` definition inside a ranked `#[decycle]` module is now accepted** and re-emitted
  in place, instead of being rejected with "macro is not supported in #[decycle] module". It is
  emitted ahead of the helper modules that carry the rewritten impls, so the macro remains usable
  inside cyclic impl bodies. (The structural engine already passed it through.) An item-position
  macro *call* is still rejected, since it may expand to impls decycle cannot see.

## [0.5.2] — 2026-08-16

> **Release status.** This section spans several releases, because the workspace version is
> bumped when a release is cut rather than per change. 0.5.0 and 0.5.1 were published from it
> (0.5.1 on 2026-08-08, from commit `a085766`); no release has been yanked, and there is no 0.4.x.
>
> **New in 0.5.2:** the *Fixed — correctness (2026-08-15 audit)* entries below, plus three changes
> that were committed after the 0.5.1 publish and so never shipped — the doubled `?Sized`
> relaxation fix, the `__DecycleBody` where-predicate restatement, and the value-level
> instantiation-growth rejection.
>
> **This is a patch-numbered release that contains behavior breaks.** Under Cargo's rules a
> `0.5.1 → 0.5.2` bump is treated as compatible, so `cargo update` will pull it into existing
> builds; several inputs that previously compiled are now rejected. They are listed under
> *Breaking in 0.5.2* below.

### Breaking in 0.5.2

Each of these previously compiled and now does not. All are narrow, and each error names the fix.

- **A `#[decycle]` trait may no longer declare a defaulted generic parameter** (`trait Ca<T = u8>`).
  Only the *omitted-argument* spelling was actually broken; declaring a default and writing the
  argument at every use site worked. The rejection is therefore broader than the defect, and was
  chosen because the marker and fn-pointer alias place trait generics and method generics in one
  parameter list, where a carried-through default is not expressible (`generic parameters with a
  default must be trailing`). Spell the parameter at every use site instead.
- **`marker` is now genuinely required** on a standalone `#[decycle]` trait naming a relative type
  path, as the documentation always claimed. It was previously accepted and silently ignored,
  because the interning pass never ran.
- **Arguments on a `#[decycle(...)]` attached to an item *inside* a `#[decycle]` module are now
  rejected** rather than discarded. Engine options belong on the enclosing `mod`; writing
  `#[decycle(structural)]` on an inner trait previously selected the ranked engine in silence.
- **Structural: a method generic whose bound mentions `Self`** (`F: Fn(&Self)`) is rejected. It
  could never compile; use `fn(&Self)`, `&dyn Fn(&Self)`, or the ranked engine.

### Fixed — correctness (2026-08-15 audit)

Three of these were **silent** rather than an error, which is the class this section is most
concerned with: `super::` resolving to the wrong item, the obligation-graph API reporting no
cycle for a cyclic module, and `#[decycle(structural)]` on an inner item selecting the ranked
engine. The rest are valid programs that failed to compile, or diagnostics that misdirected.

- **`super::` in expression position resolved to the wrong item (ranked).** `LiftRelative`
  lifted `super::` paths in type, trait-bound and qself position but left plain value paths
  "to resolve normally" — except adopted impls are re-emitted one or two modules deeper, so
  `super::` then named the shadowing module instead of the intended parent. The path still
  resolved, to a *different* item: a body calling `super::helper()` silently got the module's
  own `helper` (1) instead of the crate root's (100), with no error or warning. Value paths
  are now lifted through the same `__DecycleRelMod_*` alias (struct literals and patterns
  included); the two shapes that cannot be lifted — a relative path inside a macro invocation,
  and a `use super::…;` in a method body — are rejected rather than left wrong.
- **The growth check rejected valid programs.** Call sites were matched on method *name*
  alone, with arguments mapped by position in the caller's own signature, so a same-named
  method on an unrelated trait taking its parameter by reference was reported as "reborrows
  its own by-value generic parameter". The code compiled and ran fine without `#[decycle]`,
  and there was no escape hatch short of renaming the other trait's method. A flagged call
  must now resolve to the cycle's own trait method. Fixing the argument offset also exposed a
  true positive the old mapping had been hiding (UFCS calls were off by one).
- **A foreign type sharing a cycle head's last path segment injected a sibling's premises.**
  `crate::other::B: Tr` was read as the local member `B`, so an unrelated impl's `where`
  bounds were unioned into the cycle's impl — renaming the foreign type made the identical
  program compile. Bound targets are now matched with the bare-or-`self::` rule the peel and
  contract passes already used. The same root cause turned one intended abort into a
  138-line `TrRanked` wall.
- **The obligation-graph API dropped nested participants** when a bound's head was itself a
  participant, so `cyclic_subgraph` returned an *empty* cycle set for a fully cyclic module and
  the documented analyze → subgraph → process workflow silently emitted no terminators.
- **A cross edge to a generic cyclic method is now registered** instead of always failing
  closed at the floor. A generic method reached from a non-generic caller — where nothing in
  scope can name the type argument — still fails closed, as documented.
- **`where Self: OtherTrait` lost its cross-edge registration**, because the target stayed a
  literal `Self` and never unified. It is now resolved to the impl's own self type, which also
  removes a latent `E0411` in the generated register-once fn.
- **`size_of::<Self>()` was folded into the re-entry key unconditionally**, so an
  `impl<T: ?Sized>` failed with an `E0277` spanned at the attribute, citing a std internal and
  advising the user to remove `?Sized` from their own API. The fold is now gated on a real
  sizedness check.
- **Raw identifiers panicked the macro.** `struct r#loop` or `mod r#match` produced
  `"__DecycleNat_r#loop_m" is not a valid identifier`.
- **The `marker` / type-interning path for a standalone `#[decycle]` trait was dead.**
  `reduce_roots()` was never called, so nothing was ever interned, `marker` was accepted and
  ignored, and its own "specify `marker`" abort was unreachable; the emitted trait was also
  snapshotted before the interning rewrite. A trait naming a crate-local type now works.
  Consequently `marker` is now genuinely *required* where the docs already said it was.
- **Prelude trait bounds were rejected on a standalone `#[decycle]` trait** with a bare
  "use absolute path": the leaker config never allowed the std prelude. `T: Clone` works now.
  (Bounds with arguments, e.g. `Into<usize>`, must still be spelled absolutely — that
  comparison lives in `type-leak`.)

Structural engine:

- **Associated consts were ambiguous inside a cyclic body** (`E0034`), despite being listed as
  supported: `__DecycleBody` declared them alongside the real trait. Only associated *types*
  are declared there now — and those turn out to be load-bearing, not merely harmless.
- **An impl spelling the concrete self type where the trait says `Self`** — legal Rust — failed
  with `E0053` whose "help" told the user to write `__ATerm_<nonce>`. Such signatures are now
  canonicalised against the trait declaration, and where the declaration is not available
  (a `use`-imported trait) the error names the actual fix.
- **A method generic whose bound mentions `Self`** (`F: Fn(&Self)`) could never compile, and
  two of the engine's own diagnostics recommended it. It is rejected with a clear message and
  those diagnostics now point at `fn(&Self)` / `&dyn Fn(&Self)`.
- **Method attribute macros expanded twice**, contradicting the documented contract:
  `#[tracing::instrument]`-style attributes produced two nested spans per call, and an
  attribute emitting a named item failed outright with `E0407`.
- **`#[decycle(...)]` arguments on an item *inside* a `#[decycle]` module were silently
  discarded** — including `structural`, so a user who annotated the inner trait instead of the
  module silently got the ranked engine. They are now rejected.

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
  detection no longer parses the consumer's `Cargo.toml`).
