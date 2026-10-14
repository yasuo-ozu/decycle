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
decycle = "0.6.0"
```

### Cargo features

All are on by default; a normal `std` user needs none of these flags.

| feature | default | what turning it **off** does |
| --- | --- | --- |
| `std` | on | Drops the re-entry registry, which is a `thread_local!` map. The ranked engine then requires `support_infinite_cycle = false`; asking for unbounded re-entry without `std` is a compile error naming `UnboundedReentryRequiresTheDecycleStdFeature`. The structural engine is unaffected — it emits no runtime machinery at all. |
| `api` | on | Drops the programmatic surface (`decycle::{analysis, ranked, structural, safegraph}`). This is what keeps `decycle-impl` out of your *target* graph, and is therefore required for `no_std`. |
| `announce` | on | Drops the re-export `decycle::announce`, through which a trait **outside** a `#[decycle]` module is announced for other crates to `#[decycle] use` (`#[decycle::announce::announce(crate = ::decycle::announce)]`), and the [announce](https://docs.rs/announce) dependency that `decycle` itself has for it. It does not take announce out of the graph while `api` is on: `decycle-impl`, which `api` brings in, depends on announce with its `proc-macro-api` feature. Unlike `api`, it builds for a core-only target (announce is taken without that feature), so a `no_std` crate that announces traits keeps it on. `#[decycle] use` of an announced trait, `decycle::check` and everything inside a `#[decycle]` module need no feature. |

**`no_std` needs `default-features = false`.** With the defaults on, `api` pulls `decycle-impl`
(and `proc-macro2`, and announce's `proc-macro-api`) into the target graph, so a core-only target
fails to build. The working spelling is:

```toml
[dependencies]
decycle = { version = "0.6.0", default-features = false }
# ... or, to announce traits too (`#[announce]`, `decycle::check`) from the `no_std` crate:
# decycle = { version = "0.6.0", default-features = false, features = ["announce"] }
```

Note that turning `api` off does **not** shrink a normal native build: `decycle-macro` depends on
`decycle-impl` unconditionally, so it is compiled for the host either way. The feature exists to
keep it off the *target*, which is what makes cross-compiling to a core-only target work.

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
            // same
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
            // same
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
the module. Such a trait is announced with [`#[announce]`](https://docs.rs/announce)
(re-exported as `decycle::announce`), not annotated with `#[decycle]`:

```rust
# use decycle::decycle;

// can be defined out of the crate
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
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
**ranked** engine  or the **structural** unroll — `#[decycle(structural)]`.

| | `#[decycle]` (ranked) | `#[decycle(structural)]` |
| --- | --- | --- |
| Deep-recursion **runtime cost** | not zero-cost | **zero-cost** |
| **Unbounded depth** | only when `support_infinite_cycle = true` | always |
| Re-entry across **several instantiations** of a generic method | **✓** | ✗ |
| **`no_std`** | only when `support_infinite_cycle = false` *and* `default-features = false` | **✓** with `default-features = false` |
| **`#[track_caller]`** on a cyclic method | only when `support_infinite_cycle = false` (the fn-pointer indirection loses the caller location past the floor) | **✓** |
| Arg mentioning `Self`: **`impl Fn(&Self)`** (APIT) | **✓** — but see the row below | ✗ (use generics) |
| Generic arg instantiated with an **anonymous type** (closure, `async` block, `-> impl Trait` value) | ✗ in unbounded mode — see the closure note below | **✓** |
| Arg mentioning `Self`: **`fn(&Self)`** / **`&dyn Fn(&Self)`** | ✗ | **✓** |
| Non-`#[decycle]` **supertrait** on the trait | **✓** | ✗ |
| Third-party trait *in* the cycle | only when announced (`#[announce]`) at its definition | **✓**  |

> **Note on closures (ranked engine, unbounded mode only).** With
> `support_infinite_cycle = true` (the default), instantiating a cyclic method with a closure or
> `async` block — any `impl Fn(..)` or other generic argument, whether or not it mentions `Self` —
> is **rejected at runtime** with a panic whose message contains "anonymous type". The rejection is
> deterministic and fires on the method's first call, however shallow. Named functions work as-is;
> a non-capturing closure works once coerced to a function pointer:
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
> # fn main() {
> use fold_m::Fold;
> // Rejected at runtime (panic: "... anonymous type ..."): a closure.
> // fold_m::A.fold(|v| v + 7, 25);
> // Works, at any depth.
> assert_eq!(fold_m::A.fold((|v| v + 7) as fn(usize) -> usize, 25), 32);
> # }
> ```
>
> A closure that *captures* cannot be coerced to a `fn` pointer — pass its captures as ordinary
> arguments instead, or use bounded mode. The restriction does **not** apply with
> `support_infinite_cycle = false`, which accepts closures unchanged, nor to the structural engine
> (which rejects `impl Fn(&Self)` for its own, unrelated reason — see the table).
>

## How the algorithms work?

<details>

<summary>Ranked traits</summary>

**The idea.** Every trait in the cycle gets a hidden "Ranked" twin carrying one extra type
parameter, which stands for remaining recursion depth. Your impls are duplicated against that
parameter, and calls go through the twin at the current depth. Because each rank refers only to
the rank below it, the obligation is no longer circular — the compiler can solve it.

That chain has to end somewhere; the last rank is the **floor**. What happens there is what
`support_infinite_cycle` selects, and it is the whole difference between the two modes below.

Smallest example (two mutually recursive traits):

```rust
# use decycle::decycle;

#[decycle::announce::announce(crate = ::decycle::announce)]
trait A { fn a(&self) -> ::core::primitive::usize; }
#[decycle::announce::announce(crate = ::decycle::announce)]
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
# fn main() {}
```

**Past the floor (`support_infinite_cycle = true`, the default).** The deepest rank does not
stop. It re-enters the *original* trait impl at full height through a type-erased fn pointer,
so recursion depth is bounded only by the OS stack, like any recursive-descent code. (A
genuinely non-terminating cycle therefore overflows the stack rather than being cut off.)

The pointer lives in a **thread-local** registry. Every inductive frame registers what it and
its cyclic-bound siblings need before descending — on the same call stack, hence the same
thread — so each thread's lookups find their own targets, independently, for any cycle width
at any `recurse_level >= 1`. Generic methods work too: each instantiation gets its own entry.

**What it costs.** A few thread-local map inserts per inductive frame, and one lookup per floor
crossing — that is, once every `recurse_level` levels of real recursion, not once per call.

**Where it stops instead.** A few shapes cannot be registered. Each fails closed with an
actionable panic that cannot corrupt or poison other cycles or threads:

- a method instantiated with a **closure** or `async` block (see the closure note under
  *Two algorithms*);
- a **generic method** whose floor is reached before any frame of that instantiation ran on
  this thread, *and* whose instantiation no frame on the way down can name. A cross edge from a
  caller that declares the same generics is registered from that caller's prologue, so it works;
  what still fails closed is a generic method reached from a **non-generic** caller, where the
  type argument is chosen inside the body and nothing in scope can spell it;
- an impl whose cyclic bound targets a **bare type parameter**
  (`impl<T: Cb> Ca for Wrap<T>`), or a heterogeneous side-bound cycle whose registering impl
  does not syntactically cover what a reachable sibling needs. Such a cycle is still unbounded
  through its other impls.

**At the floor (`support_infinite_cycle = false`).** No runtime machinery is emitted at all —
this is the zero-cost mode. Recursion simply stops at the configured `recurse_level`, panicking
with `unimplemented!` if it is exceeded.

</details>


<details>
<summary> The structural unroll (`#[decycle(structural)]`) </summary>

A second, self-contained algorithm that emits **no runtime machinery** —
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

**The idea.** Each cycle-member type `Xxx` gets a `#[repr(transparent)]` twin
`__XxxTerm(pub Xxx)` — same layout, same visibility, but a *different type* as far as the
trait solver is concerned. The impl moves onto the twin, and the cyclic `where`-bounds are
stripped from it, which is what breaks the obligation. Your natural type then delegates to the
twin through a same-layout cast.

**Why your code still reads the same.** The method body is not rewritten. It is placed
verbatim on a private local trait implemented for the *natural* type, so `self`, `Self` and
constructors mean exactly what they did before. The one bare cyclic bound that remains lives
there, where it resolves on sight and still pins generics that would otherwise be uninferable.

Cross-trait cycles (`Expr: Eval → Expr: Size → Expr: Eval`) are found with a
`(type, trait)`-pair obligation graph, so they work the same way.

The expansion of the example above (simplified, real internal names):

```rust,ignore
mod ast {
    #[inline]
    unsafe fn __decycle_cast<A, B>(a: A) -> B {
        // `a` is parked in `ManuallyDrop` and never moved again, so the bitwise copy is the
        // only owner. (Moving `a` into a `forget` *after* the copy would retag any `Box`
        // inside it and invalidate the value just produced.)
        let a = ::core::mem::ManuallyDrop::new(a);
        ::core::mem::transmute_copy::<::core::mem::ManuallyDrop<A>, B>(&a)
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
shape (`&self`, `&mut self`, owned `self`, `self: Box<Self>`); method generics (but a method
generic whose *bound* mentions `Self`, such as `F: Fn(&Self)`, is rejected — use `fn(&Self)` or
`&dyn Fn(&Self)`, or the ranked engine);
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

## Traits outside a module

A trait defined outside a `#[decycle]` module (another module, another crate) is announced with
[`#[announce]`](https://docs.rs/announce/latest/announce/attr.announce.html), which `decycle`
re-exports as `decycle::announce`, and brought in with `#[decycle] use`. `#[decycle]` on such a
trait does the same: it expands to `#[announce(crate = ::decycle::announce)]` (with
`decycle = path`, `crate = path::announce`). Write `#[announce]` itself for its options:

```rust
# use decycle::decycle;
// `crate = ..` is needed when the crate does not depend on `announce` itself.
#[decycle::announce::announce(crate = ::decycle::announce, decycle::check)]
pub trait Walk {
    fn walk(&self, depth: usize) -> usize;
}

#[decycle]
mod cycle {
    #[decycle]
    use super::Walk;

    pub struct A(pub Option<Box<B>>);
    pub struct B(pub Option<Box<A>>);

    impl Walk for A where B: Walk {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |b| b.walk(depth + 1))
        }
    }
    impl Walk for B where A: Walk {
        fn walk(&self, depth: usize) -> usize {
            self.0.as_ref().map_or(depth, |a| a.walk(depth + 1))
        }
    }
}
# fn main() {}
```

`decycle::check` is optional. It validates the trait at its definition, so that the error appears at
the trait instead of where a module receives it. It rejects a return-position `impl Trait` and an
`async fn` in a method (neither engine can carry the opaque return type through a cycle), and a
default on a generic parameter (`trait Tr<T = u8>`). The last is a rule of the default **ranked**
engine only, and `decycle::check` enforces the ranked engine's rules; for a trait that is only meant
for `#[decycle(structural)]`, which accepts a default, write `decycle::check(structural)` to skip it.

`#[announce]` checks that every path in the trait means the same thing in another crate, and
rejects a path relative to the defining module (`Local`, `super::X`, `std::fmt::Debug`) at the
definition: write it absolutely (`crate::Local`, `::core::fmt::Debug`). Primitive types need
nothing, and `allow_std_prelude = true` accepts the bare std prelude names (`String`, `Vec<u8>`,
`Clone`). The error also suggests announce's `repeater`; `#[decycle] use` does not support it,
so write the path absolutely. A trait written inside a
`#[decycle]` module is not carried through a macro, so none of this applies to it.

The options of 0.5's `#[decycle]` on a trait map to:

| 0.5 `#[decycle(..)]` on a trait | now |
| --- | --- |
| `#[decycle]` | `#[decycle]` (expands to `#[announce(crate = ::decycle::announce)]`) |
| `alter_macro_name = N` | `#[announce(.., macro_name = N)]`, or `mock = path`; a module then writes both `use super::d::Walk;` and `#[decycle] use super::d::WalkMac;` (below) |
| `decycle = path` | unchanged: the trait is announced with `crate = path::announce` |
| `marker = path` | none: write the paths of the trait absolutely (`crate::..`, `::krate::..`); announce reports the ones that do not resolve |
| (new) | `decycle::check` (`decycle::check(structural)`): validate the trait at its definition |

A trait announced with `macro_name = N` has a trampoline `N!` that is not named like the trait (so
that a derive or attribute macro can keep the trait's name). The module's `impl Trait for ..` is
matched to the trait by name, and the trait is brought in by a plain `use`, beside the
`#[decycle] use` of its trampoline (the two live in different namespaces):

```text
use super::d::Walk;                  // the trait
#[decycle] use super::d::WalkMac;    // its trampoline
```

A `#[decycle] use` without an `as` does not say which trait it carries, so decycle asks the
trampoline (`Walk`) when the module has an `impl` of a trait that no `#[decycle]` item names but a
plain `use` does. That is one more trampoline call at the expansion of such a module, and no other
module pays it. The plain `use` must be written in the module itself; a glob import (`use super::*;`)
is not looked at.

Do not write `#[decycle] use super::d::WalkMac as Walk;` when a derive or attribute macro named
`Walk` shares the path: `use super::d::Walk;` imports the trait **and** that macro, and a `use`
cannot import only one of the namespaces of a name, so `as Walk` would define `Walk` twice in the
macro namespace (E0252). The `as` form is for a trampoline whose name is free to take.

### Stand-ins for traits you do not own

`#[announce(mock = path::to::RealTrait)]` declares a trait a *stand-in* for the real trait at
`path` (e.g. one you do not own). Only its trampoline macro is emitted, at the stand-in's path;
`#[decycle] use` sites treat the carried copy as the real trait, so their impls implement `path`.
Name the stand-in like the real trait, or `#[decycle] use stand::X as Real;` when its name differs
(a stand-in `use`d under another name without the `as` is an error, in both engines). A stand-in
written by hand imports nothing but its trampoline, so the module also `use`s the real trait under
that name. [announce-std](https://docs.rs/announce-std) provides such copies for the traits of `std`,
and they work with `#[decycle] use` as they are: `announce_std::fmt::Display` is the real trait in
the type namespace as well as the trampoline, so `#[decycle] use announce_std::fmt::Display;` is all
a module writes, and `use core::fmt::Display;` beside it is an error (`Display` defined twice).

## Contributing

Contributions are welcome. Please open an issue or PR.

## License

MIT
