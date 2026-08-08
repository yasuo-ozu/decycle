<p align="center">
  <img src="https://raw.githubusercontent.com/yasuo-ozu/decycle/main/decycle.png" width="140" alt="decycle logo: a broken cycle escaping on a tangent">
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

```toml
[dependencies]
decycle = "0.5.0"
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
use decycle::decycle;

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

    impl Evaluate for Term
    where
        Expr: Evaluate,
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
use decycle::decycle;

#[decycle]
pub trait Evaluate {
    fn evaluate(&self, input: &[&'static str], index: &mut usize) -> i32;
}

#[decycle]
#[allow(dead_code)]
mod cycle {
    #[decycle]
    use super::{Evaluate};

    // ...
}
fn main() {}
```

## Two algorithms

`#[decycle]` on a module selects one of two independent cycle-breaking strategies: the default
**ranked** engine  or the **structural** unroll — `#[decycle(structural)]`.

| | `#[decycle]` (ranked) | `#[decycle(structural)]` |
| --- | --- | --- |
| Deep-recursion **runtime cost** | not zero-cost (vtable hop) | **zero-cost** |
| **Unbounded depth** | only when `support_infinite_cycle = true` | always |
| Re-entry across **several instantiations** of a generic method | **✓** (fn-pointer re-entry) | ✗ (layout cast can't) |
| Genuinely **growing** type argument (one wrapper per level) | ✗ — see note below | ✗ |
| **`no_std`** | only when `support_infinite_cycle = false` | **✓** |
| Arg mentioning `Self`: **`impl Fn(&Self)`** (APIT) | **✓** — but see the row below | ✗ (use generics) |
| Generic arg instantiated with an **anonymous type** (closure, `async` block, `-> impl Trait` value) | ✗ in unbounded mode — rejected at runtime; a named fn or `fn` pointer works. Not a pending fix — see the closure note below | **✓** |
| Arg mentioning `Self`: **`fn(&Self)`** / **`&dyn Fn(&Self)`** | ✗ | **✓** |
| **`#[track_caller]`** on a cycle method | ✗ in unbounded mode (clean compile error) / **✓** when `support_infinite_cycle = false` | **✓** |
| Non-`#[decycle]` **supertrait** on the trait | **✓** | ✗ |
| Third-party trait *in* the cycle | only when `#[decycle]`-annotated at its definition | **✓**  |
> **`no_std`.** Turn off default features (`decycle = { version = "..", default-features = false }`).
> That drops two things: the `std` feature, which carries the unbounded re-entry registry (a
> `thread_local!` map), and the `api` feature, which carries the programmatic surface and is what
> pulls `decycle-impl` into your target build. What remains is the attribute macro plus a
> `core`-only runtime — the structural engine emits nothing else, and needs no `alloc` either.
>
> The ranked engine still works at a fixed depth; only `support_infinite_cycle = true` (its
> default) needs the registry, and asking for it without `std` is a compile error naming the
> feature rather than a broken path into crate internals.

> **Note on "growing" type arguments.** Neither engine can make a *genuinely* growing recursion work —
> one whose instantiation strictly grows every level, e.g. `A<Vec<X>>: Tr` on `impl<X> Tr for A<X>`, or a
> `Dup<Dup<…>>` stream tower threaded unerased. That is not an engine limitation but a language one:
> the instantiation family is infinite, so rustc stops it regardless — the trait solver overflows
> (`E0275`) or the monomorphizer reports *"reached the recursion limit while instantiating"*. The ranked
> engine rejects such a cyclic bound up front rather than diverging.
>
> What the ranked engine *does* add over structural is re-entry across a **finite set** of
> instantiations past the recursion floor. The way to make a growing tower finite is to **erase it at
> the recursion boundary** — pin the stream to one fixed `&mut dyn Trait` layer — after which the cycle
> works under *both* engines (`tests/dyn_stream_reentry.rs`).

> **Note on closures (ranked engine, unbounded mode only).** The unbounded re-entry registry keys
> each method instantiation by `type_name`, and rustc renders **every** closure and `async` block
> in a function as `{{closure}}`, with no disambiguator — two closures in one function would share
> a key, and the recursion floor would call the wrong one. So under
> `support_infinite_cycle = true` (the default), instantiating a cyclic method with a closure or
> `async` block — any `impl Fn(..)` or other generic argument, whether or not it mentions `Self` —
> is **rejected at runtime** with a panic whose message contains "anonymous type". The rejection is
> deterministic and fires on the method's first call, even when recursion never reaches the floor.
> Named functions work as-is; a non-capturing closure works once coerced to a function pointer,
> which is a uniquely named type:
>
> ```rust
> use decycle::decycle;
>
> #[decycle]
> mod fold_m {
>     #[decycle]
>     pub trait Fold {
>         fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize;
>     }
>     pub struct A;
>     impl Fold for A
>     where
>         A: Fold,
>     {
>         fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize {
>             if n == 0 { f(0) } else { A.fold(f, n - 1) + 1 }
>         }
>     }
> }
>
> fn main() {
>     use fold_m::Fold;
>     // Rejected at runtime (panic: "... anonymous type ..."): a closure.
>     // fold_m::A.fold(|v| v + 7, 25);
>     // Works, at any depth: a function pointer is uniquely named.
>     assert_eq!(fold_m::A.fold((|v| v + 7) as fn(usize) -> usize, 25), 32);
> }
> ```
>
> A closure that *captures* cannot be coerced to a `fn` pointer — pass its captures as ordinary
> arguments instead, or use bounded mode. The restriction does **not** apply with
> `support_infinite_cycle = false`: bounded mode emits no registry and accepts closures unchanged.
> It also does not apply to the structural engine (which rejects `impl Fn(&Self)` at compile time
> for its own, unrelated reason — see the table).
>

## How the algorithms work?

<details>

<summary>Ranked traits</summary>

`#[decycle]` rewrites the annotated module into a set of ranked helper traits. Each
original trait gets a hidden "Ranked" version that carries an extra type parameter
representing recursion depth. Implementations are duplicated with that rank parameter, and
calls are delegated through the ranked trait for the current depth. This breaks the direct
cycle at the type level. Past the floor it can re-enter through a type-erased fn pointer, which lets
one cycle serve **several instantiations** of a generic method — so prefer it unless you specifically
want zero runtime machinery. (It does not make a genuinely *growing* type argument work; nothing can —
see the note above.)

Smallest example (two mutually recursive traits):

```rust
use decycle::decycle;

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
fn main() {}
```

Expected expansion (simplified, with stable names):

```rust
trait A { fn a(&self) -> usize; }
trait B { fn b(&self) -> usize; }

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
fn main() {}
```

When `support_infinite_cycle = true` (the default), the deepest rank (the "floor")
does not stop: it re-enters the *original* trait impl at full height through a
type-erased fn pointer held in a **thread-local** registry, keyed by the
`type_name` of a generated per-(trait, method, instantiation) marker type plus a
layout fingerprint of the keyed types. A key is only sound when that string names
exactly one type, and `type_name` is not injective for anonymous types — rustc
renders every closure and `async` block declared in one fn as `{{closure}}`, with
no disambiguator — so a method instantiated with a closure or `async` block is
rejected at runtime on its first call with a panic mentioning "anonymous type"
(pass a named function or coerce to a function pointer; see the closure note
under *Two algorithms*). Every inductive frame idempotently
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

</details>


<details>
<summary> The structural unroll (`#[decycle(structural)]`) </summary>

A second, self-contained algorithm that emits **no runtime machinery** —
everything is resolved at compile time.

```rust
use decycle::decycle;

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

**Limitations.** The headline: the layout cast is monomorphic, so a cycle cannot serve **several
instantiations** of a generic method — that is what the ranked engine's fn-pointer re-entry is for. (An
unerased `Dup<…>`-style stream tower is out of reach for both engines; erase it to a fixed
`&mut dyn Trait` and structural handles it fine — `tests/dyn_stream_reentry.rs`.) Return-position
`impl Trait` and `async fn` are rejected
up-front in both engines (declare an associated type, or return a boxed type). See
[Two algorithms](#two-algorithms) for the full per-engine breakdown of what each side can't do.

</details>

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
