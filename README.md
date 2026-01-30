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
decycle = "0.1.0"
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
  - `recurse_level`: expansion depth (default 10)
  - `support_infinite_cycle`: enables/disable infinite cycle handling (default true)
  - `decycle`: override the path used to refer to this crate
- **Trait**:
  - `#[decycle::decycle(marker = path, decycle = path)]`
  - `marker`: marker type used for internal references (required when reported)
  - `decycle`: override the path used to refer to this crate

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
    impl<SelfT: ARanked<(((((((()),),),),),),)>> A for SelfT {
        fn a(&self) -> usize { <SelfT as ARanked<(((((((()),),),),),),)>>::a(self) }
    }
    impl<SelfT: BRanked<(((((((()),),),),),),)>> B for SelfT {
        fn b(&self) -> usize { <SelfT as BRanked<(((((((()),),),),),),)>>::b(self) }
    }

    // Ranked impls for the concrete types.
    impl<Rank> ARanked<(Rank,)> for Left where Right: BRanked<Rank> {
        fn a(&self) -> usize { self.0 + 1 }
    }
    impl<Rank> BRanked<(Rank,)> for Right where Left: ARanked<Rank> {
        fn b(&self) -> usize { self.0 + 1 }
    }
}
# fn main() {}
```

When `support_infinite_cycle = true` (the default), decycle also emits a small
runtime shim that caches function pointers in `OnceLock` to allow cycles beyond
the compile-time recursion limit. When it is `false`, decycle stops at the
configured `recurse_level` and uses `unimplemented!` once the limit is reached.
(the runtime sim is non-zero-cost)

### Coinduction

Decycle is often compared with the [coinduction](https://crates.io/crates/coinduction)
crate and its documentation on [docs.rs](https://docs.rs/coinduction), which is
developed to solve the same problem. Coinduction’s
expands all related dependencies into a flat set, which removes
the dependency loop at the type level and lets mutually recursive bounds resolve
in one pass.
