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

> **Advisory:** versions prior to 0.4.0 (including 0.3.0) have a broken
> `support_infinite_cycle = true` (the default): any recursion deeper than
> `recurse_level` jumps through an incorrectly-transmuted pointer and crashes
> (SIGSEGV). Upgrade to 0.4.0 or later, or set `support_infinite_cycle = false`
> on older versions. See `CHANGELOG.md` for details.

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
trait A {
    fn a(&self) -> ::core::primitive::usize;
}

#[decycle]
trait B {
    fn b(&self) -> ::core::primitive::usize;
}

#[decycle]
mod cycle {
    #[decycle]
    use super::{A, B};

    struct Left(usize);
    struct Right(usize);

    impl A for Left
    where
        Right: B,
    {
        fn a(&self) -> usize {
            self.0 + 1
        }
    }

    impl B for Right
    where
        Left: A,
    {
        fn b(&self) -> usize {
            self.0 + 1
        }
    }
}

# fn main() {}
```

## Attribute Arguments

- **Module**: 
  - `#[decycle::decycle(recurse_level = N, support_infinite_cycle = true|false, decycle = path)]`
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

## The mechanism

`#[decycle]` rewrites the annotated module into a set of ranked helper traits.
Each original trait gets a hidden "Ranked" version that carries an extra type
parameter representing recursion depth. Implementations are duplicated with that
rank parameter, and calls are delegated through the ranked trait for the current
depth. This breaks the direct cycle at the type level.

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

    // Floor: the compile-time chain bottoms out here (see "The mechanism"
    // below for what actually happens here instead of `unimplemented!`).
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

### Coinduction

Decycle is often compared with the [coinduction](https://crates.io/crates/coinduction)
crate and its documentation on [docs.rs](https://docs.rs/coinduction), which is
developed to solve the same problem. Coinduction’s
expands all related dependencies into a flat set, which removes
the dependency loop at the type level and lets mutually recursive bounds resolve
in one pass.
