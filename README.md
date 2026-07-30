<p align="center">
  <img src="https://raw.githubusercontent.com/yasuo-ozu/decycle/main/assets/logo.svg" width="140" alt="decycle logo: a broken cycle escaping on a tangent">
</p>

# Decycle

[![Crates.io](https://img.shields.io/crates/v/decycle.svg)](https://crates.io/crates/decycle)
[![Documentation](https://docs.rs/decycle/badge.svg)](https://docs.rs/decycle)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Attribute macros for resolving circular trait obligations in Rust.

## Overview

Decycle provides a single attribute macro, **`#[decycle]`**, that rewrites a
module to break mutually recursive trait bounds. It lets you define types and
traits with circular dependencies that would otherwise fail to compile.

## Quick Start

Add this to your `Cargo.toml`:

```toml
[dependencies]
decycle = "0.4.0"
```

## Why Decycle?

Without decycle, circular trait obligations cause compilation errors. Here's
what happens when trying to create a simple calculator parser:

```rust,compile_fail
trait Evaluate {
    fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32;
}

pub struct Expr;
pub struct Term;

// ERROR: Cannot prove Term: Evaluate
impl Evaluate for Expr
where
    Term: Evaluate,  // Expr depends on Term...
{
    fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32 {
        let left_val = Term.evaluate(input, index);
        let op = input[*index];
        *index += 1;
        let right_val = Term.evaluate(input, index);
        match op {
            "+" => left_val + right_val,
            "-" => left_val - right_val,
            _ => left_val,
        }
    }
}

// ERROR: Cannot prove Expr: Evaluate
impl Evaluate for Term
where
    Expr: Evaluate,  // ...and Term depends on Expr!
{
    fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32 {
        let token = input[*index];
        *index += 1;
        if token == "(" {
            let result = Expr.evaluate(input, index);
            *index += 1; // skip closing ')'
            result
        } else {
            token.parse::<i32>().unwrap()
        }
    }
}
```

The `#[decycle]` macro solves this by breaking the circular dependency cycle.

## Examples

### Basic Circular Dependencies

This example shows how to break circular trait dependencies using `#[decycle]`:

```rust
# use decycle::decycle;
#[decycle]
mod calculator {
    #[decycle]
    pub trait Evaluate {
        fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32;
    }

    pub struct Expr;
    pub struct Term;

    impl Evaluate for Expr
    where
        Term: Evaluate,
    {
        fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32 {
            // ...
            # let left_val = Term.evaluate(input, index);
            # let op = input[*index];
            # *index += 1;
            # let right_val = Term.evaluate(input, index);
            # match op {
            #     "+" => left_val + right_val,
            #     "-" => left_val - right_val,
            #     _ => left_val,
            # }
        }
    }

    impl Evaluate for Term
    where
        Expr: Evaluate,
    {
        fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32 {
            // ...
            # let token = input[*index];
            # *index += 1;
            # if token == "(" {
            #     let result = Expr.evaluate(input, index);
            #     *index += 1; // skip closing ')'
            #     result
            # } else {
            #     token.parse::<i32>().unwrap()
            # }
        }
    }
}

fn main() {
    use calculator::Evaluate;
    let input = vec!["2", "+", "3"];
    let mut index = 0;
    assert_eq!(calculator::Expr.evaluate(&input, &mut index), 5);
}
```

### Using `use` to mark traits

You can also annotate `use` items inside the module to use traits defined out of
the module:

```rust
# use decycle::decycle;
#[decycle]
pub trait Evaluate {
    fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32;
}

#[decycle]
##[allow(dead_code)]
mod cycle {
    #[decycle]
    use super::{Evaluate};

    // ...
}
# fn main() {}
```

## Two algorithms

`#[decycle]` on a module selects one of two independent cycle-breaking strategies: the default
**ranked** engine (hidden "Ranked" helper traits + a thread-local runtime re-entry registry) or the
**structural** unroll — `#[decycle(structural)]` — using per-type `#[repr(transparent)]` terminators
and layout casts with no runtime. They break the same cycles and are interchangeable for ordinary
method recursion (about nine test files run identical scenarios under both, via
`tests/common/mod.rs`), but differ sharply in cost and in what they can't do. Every cell below is
verified by a compiled spike (✓ = works, ✗ = rejected / fails to compile):

| | `#[decycle]` (ranked) | `#[decycle(structural)]` |
| --- | --- | --- |
| Deep-recursion **runtime cost** | not zero-cost (vtable hop) | **zero-cost** |
| **Unbounded depth** | needs `support_infinite_cycle` option | always |
| **Growing-type-arg** recursion (`Dup<…>` tower) | **✓** (fn-pointer re-entry) | ✗ (layout cast can't) |
| Return-position **`impl Trait`** | ✗ (declare an associated type) | ✗ (declare an associated type) |
| Unsupported shape → **compile-time rejection** (caught by `cargo build`) | most shapes | **every** shape |
| Unsupported shape → **fail-closed runtime panic** (isolated floor panic) | a residual set: generic method past width, bare-param bound, heterogeneous side-bound, projection cross-edge | **never** |
| **`no_std`** | ✗ in unbounded mode (thread-local registry) | **✓** |
| Arg mentioning `Self` (**`impl Fn(&Self)`** / `dyn` / fn-ptr) | **✓** | ✗ — name the concrete type |
| Non-`#[decycle]` **supertrait** on the trait | **✓** | ✗ |
| Same-named foreign trait / finite `Wrap<u8>→Wrap<u16>` chain | **✓** (per-instantiation keys) | ✗ (name-keyed) |
| **`#[track_caller]`** | bounded ✓ / unbounded ✗ | **✓** |
| **`#[cfg]`** on a *required* trait method | ✗ (rejected up-front) | ✓ when the cfg is true |
| **`async fn`** | ✗ | ✗ |
| **`unsafe` trait** | **✓** (emits `unsafe impl`) | **✓** (emits `unsafe impl`) |
| Wrapped bound `Box<Stmt>: Tr` w/o a blanket `impl<T: Tr> Tr for Box<T>` | ✗ (clear up-front diagnostic) | ✗ (clear up-front diagnostic) |
| Third-party trait *in* the cycle, impl'd for **your** local types | ✓ if it's `#[decycle]`-annotated at its definition | **✓** (any foreign trait) |
| Cross-module cycle; foreign trait impl'd for a *foreign* type | ✗ | ✗ |
| Nested `#[decycle]` modules; glob-`use` | ✗ (rejected) | silently ignored |

**In short:**

Every unsupported shape surfaces as one of two failure modes: a **compile-time rejection** (caught by
`cargo build`) or a **fail-closed runtime panic** (an isolated floor panic when the path executes).

- **Prefer `#[decycle(structural)]`** for ordinary method-recursion cycles — it is genuinely
  zero-cost, recurses unboundedly (OS stack only), and every shape it can't handle is a
  **compile-time rejection**, never a runtime surprise.
- **Use the ranked engine** only for **growing-type-argument recursion** (a `Dup<…>` stream whose
  type grows one wrapper per descent level) — the structural layout cast can't express it. Its
  default *unbounded* mode is **not** zero-cost (per-frame thread-local inserts + a fn-pointer
  indirection that blocks inlining), and a residual set of unsupported shapes is a **fail-closed
  runtime panic** (an isolated panic at the recursion floor) rather than a compile-time rejection.

**Both engines** need the whole cycle in one *inline* `#[decycle]` module over traits **you** annotate
(ranked rejects a module with no annotated cycle; structural treats it as a silent no-op), and reject
a few shapes:

- no return-position `impl Trait` / `async fn` — declare an associated type, or return a boxed type
  (`Pin<Box<dyn Future<…>>>` for async);
- no cross-module cycle, and no foreign trait impl'd for a *foreign* type (the orphan rule blocks the
  generated impls). A third-party trait *is* allowed **in** the cycle when you `#[decycle] use` it and
  implement it for your **own** types — structural takes any such trait; ranked additionally needs it
  `#[decycle]`-annotated at its definition;
- an empty `#[decycle]` module is rejected; a nested `#[decycle]` module and a glob-`use` are rejected
  by ranked and silently ignored by structural;
- a *wrapped* cyclic bound (`Box<Stmt>: Tr`) needs its container to forward the trait — a blanket
  `impl<T: Tr> Tr for Box<T>`;
- prefer `crate::`-rooted paths over `super::` in method bodies.

An `unsafe trait` is supported by both engines (the generated impls are emitted as `unsafe impl`).

**Ranked-only:**

- `#[track_caller]` (unbounded) and `#[cfg]` on a *required* trait method — rejected;
- a `#[decycle]` trait can't be a supertrait of another; trait aliases and `Fn(..)`-sugar bounds are
  unsupported;
- unbounded mode is **std-only** and always links `type-leak`;
- its safety net is a fail-closed floor panic — a generic method first reached at cycle width >
  `recurse_level`, a bare-type-param cyclic bound (`impl<T: Cb> Ca for Wrap<T>`), a heterogeneous
  side-bound cycle, or a projection-typed cross-edge.

**Structural-only:**

- no growing-type-argument recursion (above); HRTB wrapped bounds (`for<'a> Wrap<&'a A>: Tr`) aren't
  supported either;
- an argument-position `impl Trait` / `dyn` / fn-pointer whose type mentions `Self`
  (`fn m(&self, f: impl Fn(&Self))`) isn't cast — name the concrete type;
- a non-`#[decycle]` supertrait on the decycled trait isn't supported;
- cycle membership is keyed by trait/type *name* over local ADTs only, so a same-named foreign trait
  or a finite `Wrap<u8> → Wrap<u16>` chain is misclassified — the ranked engine gets these right.

> The `type-leak` distinction is about the *generated* code (`structural` emits none); the `decycle`
> proc-macro crate always links `type-leak` at build time regardless of engine.

### The ranked engine (default)

`#[decycle]` rewrites the annotated module into a set of ranked helper traits. Each
original trait gets a hidden "Ranked" version that carries an extra type parameter
representing recursion depth. Implementations are duplicated with that rank parameter, and
calls are delegated through the ranked trait for the current depth. This breaks the direct
cycle at the type level. It is the engine that handles **growing-type-argument** recursion —
a backtracking `Dup<…>`-style stream whose type would otherwise grow one wrapper per descent
level — so prefer it unless you specifically want zero runtime machinery.

Smallest example (two mutually recursive traits):

```rust
# use decycle::decycle;

#[decycle]
trait A { fn a(&self) -> ::core::primitive::usize; }
#[decycle]
trait B { fn b(&self) -> ::core::primitive::usize; }

#[decycle]
mod cycle {
    #[decycle]
    use super::{A, B};

    struct Left(usize);
    struct Right(usize);

    impl A for Left where Right: B { fn a(&self) -> usize { self.0 + 1 } }
    impl B for Right where Left: A { fn b(&self) -> usize { self.0 + 1 } }
}
# fn main() {}
```

Expected expansion (simplified, with stable names):

```rust
# trait A { fn a(&self) -> usize; }
# trait B { fn b(&self) -> usize; }
mod cycle {
    use super::{A, B};
    struct Left(usize);
    struct Right(usize);

    // Ranked helper traits (rank parameter breaks the direct cycle).
    trait ARanked<Rank> { fn a(&self) -> usize; }
    trait BRanked<Rank> { fn b(&self) -> usize; }

    // Delegate original traits to the ranked versions at the current rank.
    // (One impl per concrete self type, not a blanket impl over a bound type
    // parameter — each keeps only its own where-bound.) The rank literal's
    // nesting depth shown here is illustrative, not the default 10.
    impl A for Left where Self: ARanked<(((((((()),),),),),),)> {
        fn a(&self) -> usize { <Self as ARanked<(((((((()),),),),),),)>>::a(self) }
    }
    impl B for Right where Self: BRanked<(((((((()),),),),),),)> {
        fn b(&self) -> usize { <Self as BRanked<(((((((()),),),),),),)>>::b(self) }
    }

    // Ranked impls for the concrete types. Each also carries a `Self: XRanked<Rank>`
    // bound (one rank lower) — that is how the induction bottoms out at the floor.
    impl<Rank> ARanked<(Rank,)> for Left where Right: BRanked<Rank>, Self: ARanked<Rank> {
        fn a(&self) -> usize { self.0 + 1 }
    }
    impl<Rank> BRanked<(Rank,)> for Right where Left: ARanked<Rank>, Self: BRanked<Rank> {
        fn b(&self) -> usize { self.0 + 1 }
    }

    // Floor: the compile-time chain bottoms out here (see the paragraph below for
    // what actually happens here instead of `unimplemented!`).
    impl ARanked<()> for Left {
        fn a(&self) -> usize { unimplemented!("decycle: cycle limit reached") }
    }
    impl BRanked<()> for Right {
        fn b(&self) -> usize { unimplemented!("decycle: cycle limit reached") }
    }
}
# fn main() {}
```

When `support_infinite_cycle = true` (the default), the deepest rank (the "floor")
does not stop: it re-enters the *original* trait impl at full height through a
type-erased fn pointer held in a **thread-local** registry, keyed by the
`type_name` of a generated per-(trait, method, instantiation) marker type plus a
layout fingerprint of the keyed types (`type_name` alone is not injective — e.g.
two closures declared in one fn share a name). Every inductive frame idempotently
registers the re-entry fns for itself and for its cyclic-bound siblings before
descending — on the same call stack, hence the same thread — so the floor's
lookup finds its target on every thread independently: for any cycle width, at
any `recurse_level >= 1`, including generic methods (each instantiation gets its
own key). Recursion depth is then bounded only by the OS stack, like any
recursive-descent code — which also means a genuinely non-terminating cycle
overflows the stack instead of being cut off. The cost is a small constant
number of lock-free thread-local map inserts per inductive frame (hoisted into
a shared per-impl helper call for everything except the frame's own
self-registration) and one lookup per floor crossing (every `recurse_level`
levels of real recursion). Three floors fail closed with an
actionable, isolated panic (it cannot corrupt or poison other cycles or
threads): a generic method's floor reached before any frame of that
instantiation ran on the current thread (e.g. a first descent at cycle width >
`recurse_level`); any floor of an impl whose cyclic bound targets a bare
type parameter (`impl<T: Cb> Ca for Wrap<T>` — its re-entry registration is not
expressible, so such a cycle is unbounded only through its other impls); and a
heterogeneous side-bound cycle where the registering impl's own bounds don't
syntactically cover every bound a reachable sibling impl needs (its
registration is skipped rather than risk naming an unprovable obligation).

When it is `false`, no runtime machinery is emitted (zero-cost) and decycle
stops at the configured `recurse_level` with an `unimplemented!` panic once the
limit is reached.

#### `impl Trait` in method arguments

Input-position `impl Trait` (APIT) in a `#[decycle]` trait method is desugared to a
method-level generic on the ranked traits — `fn m(&self, x: impl Bound)` becomes
`fn m<T: Bound>(&self, x: T)` — so it participates in ranking and re-entry like any
generic method (each instantiation gets its own registry key). Multiple APIT
parameters and HRTB bounds (`impl for<'a> Fn(&'a T)`) are supported, in both bounded
and unbounded modes. The bound may itself name a *cyclic* `#[decycle]` trait
(`fn sink(&self, other: impl Feed, ..)` where `Feed` is decycled): such an argument is
a value supplied from outside the cycle, so its bound is kept on the **public** trait
(never rank-lowered) and remains provable at the public boundary. Return-position
`impl Trait` is unsupported in **both** modes (its erased fn-pointer type is not nameable —
E0562) and produces a clean compile error pointing you at an associated type.

### The structural unroll (`#[decycle(structural)]`)

A second, self-contained algorithm with **no runtime and no `type-leak` dependency** —
everything is resolved at compile time.

```rust
# use decycle::decycle;
#[decycle(structural)]
mod ast {
    #[decycle]
    pub trait Eval {
        fn eval(&self) -> i64;
    }

    pub enum Expr {
        Lit(i64),
        Node(Option<Box<Expr>>),
    }

    impl Eval for Expr
    where
        Expr: Eval, // the cyclic obligation
    {
        fn eval(&self) -> i64 {
            match self {
                Expr::Lit(n) => *n,
                Expr::Node(c) => 1 + c.as_ref().map_or(0, |x| x.eval()),
            }
        }
    }
}

fn main() {
    use ast::Eval;
    let e = ast::Expr::Node(Some(Box::new(
        ast::Expr::Node(Some(Box::new(ast::Expr::Lit(3)))),
    )));
    assert_eq!(e.eval(), 5);
}
```

**How it works.** For each cycle-member type `Xxx` it emits a `#[repr(transparent)]`
terminator `__XxxTerm(pub Xxx)` (with the same visibility as `Xxx`). Each trait impl is
placed on the terminator via a *trait-def-inside-body* pattern — a private local trait
whose method holds the **original body verbatim**, implemented for the natural type so
`self`, `Self` and constructors resolve unchanged. The natural type's impl then delegates
to the terminator by a same-layout `transmute_copy`. The obligation cycle is broken by
**stripping the cyclic `where`-bounds** from the terminator/natural impls (a *bare* cyclic
bound is kept on the local body impl, where it resolves on-sight through the natural impl
and still pins otherwise-uninferable generics). Cross-trait cycles
(`Expr: Eval → Expr: Size → Expr: Eval`) are recognised via a `(type, trait)`-pair
obligation graph.

The expansion of the example above (simplified, real internal names):

```rust,ignore
mod ast {
    #[inline]
    unsafe fn __decycle_cast<A, B>(a: A) -> B {
        let b = ::core::mem::transmute_copy::<A, B>(&a);
        ::core::mem::forget(a);
        b
    }

    pub trait Eval { fn eval(&self) -> i64; }
    pub enum Expr { Lit(i64), Node(Option<Box<Expr>>) }

    // one #[repr(transparent)] terminator per cycle-member type
    #[repr(transparent)]
    pub struct __ExprTerm(pub Expr);

    // the body lives on the terminator, via a local trait impl'd for the *natural* type
    impl Eval for __ExprTerm {
        fn eval(&self) -> i64 {
            trait __DecycleBody: Sized { fn __run(&self) -> i64; }
            impl __DecycleBody for Expr
            where
                Expr: Eval, // bare cyclic bound kept here (resolves on-sight)
            {
                fn __run(&self) -> i64 {
                    match self { // ← ORIGINAL body, verbatim (`self`/`Self` unchanged)
                        Expr::Lit(n) => *n,
                        Expr::Node(c) => 1 + c.as_ref().map_or(0, |x| x.eval()),
                    }
                }
            }
            unsafe {
                __decycle_cast(<Expr as __DecycleBody>::__run(
                    &*(self as *const Self as *const Expr),
                ))
            }
        }
    }

    // the natural type delegates to its terminator (same layout → the cast is exact)
    impl Eval for Expr {
        fn eval(&self) -> i64 {
            unsafe {
                __decycle_cast(<__ExprTerm as Eval>::eval(
                    &*(self as *const Self as *const __ExprTerm),
                ))
            }
        }
    }
}
```

**Supported.** Self-, multi-type, multiroot and **cross-trait** cycles; every receiver
shape (`&self`, `&mut self`, owned `self`, `self: Box<Self>`); method generics;
argument-position `impl Trait`; destructured parameters; associated types/consts;
multiple `#[decycle]` traits on one type; `unsafe` traits (emitted as `unsafe impl`);
any third-party trait brought in with `#[decycle] use` and implemented for your own types;
and methods taking an already-erased `&mut dyn Trait` argument. When a *wrapped* cyclic bound
is stripped (e.g. `Box<Stmt>: Tr`) it also emits a compile-time forwarding assertion
`for<T: Tr> Box<T>: Tr`.

**Limitations.** The headline: no **growing-type-argument recursion** (a `Dup<…>`-style stream whose
type grows one wrapper per descent level — the layout cast can't express it; that's what the ranked
engine's fn-pointer re-entry is for). Return-position `impl Trait` and `async fn` are rejected
up-front in both engines (declare an associated type, or return a boxed type). See
[Two algorithms](#two-algorithms) for the full per-engine breakdown of what each side can't do.

### Comparison with `coinduction`

Decycle is often compared with the [coinduction](https://crates.io/crates/coinduction)
crate (and its [docs.rs](https://docs.rs/coinduction) documentation), which is developed
to solve the same problem. Coinduction expands all related dependencies into a flat set,
which removes the dependency loop at the type level and lets mutually recursive bounds
resolve in one pass.

## Attribute Arguments

- **Module**: 
  - `#[decycle::decycle(recurse_level = N, support_infinite_cycle = true|false, decycle = path)]`
  - `structural`: use the structural unroll instead of the ranked engine (see [Two algorithms](#two-algorithms)); incompatible with `recurse_level` / `support_infinite_cycle`
  - `recurse_level`: expansion depth (default 10, must be at least 1)
  - `support_infinite_cycle`: enables/disable infinite cycle handling (default true)
  - `decycle`: override the path used to refer to this crate
- **Trait**:
  - `#[decycle::decycle(marker = path, decycle = path)]`
  - `marker`: marker type used for internal references (required when reported)
  - `decycle`: override the path used to refer to this crate
  - `allowed_paths = [path, ...]`: overrides the type-leak allowed-path set (advanced; rarely needed)

## Contributing

Contributions are welcome. Please open an issue or PR.

## License

MIT
