//! Structural must reproduce the **generics and predicates** of what it wraps and asserts about.
//!
//! Two regressions live here. Both were found by flipping syan's `#[recurse]` suite to
//! `#[recurse(structural)]`, and both produced errors that pointed at the macro rather than at
//! anything the caller wrote — so neither would have been noticed from decycle's own suite before.

use decycle::decycle;

/// The terminator wraps the natural type, so it must be **exactly as constrained** as it.
///
/// `__ExprTerm<S>(Expr<S>)` is only well-formed if `S` carries whatever `Expr<S>` requires. The
/// terminator used to be emitted with `DeclBounds::Bare` and no `where`-clause, giving
/// `E0277 … required by a bound in Expr` against the *generated* struct.
///
/// Both spellings are covered on purpose: `S: Clone` inline and `S: Default` in the `where`-clause.
/// The inline form was the one that survived the first fix.
mod terminator_carries_predicates {
    use super::*;

    #[decycle(structural)]
    mod ast {
        #[decycle]
        pub trait Tr {
            fn total(&self) -> usize;
        }

        pub enum Expr<S: Clone>
        where
            S: Default,
        {
            Leaf(S),
            Nest(Box<Expr<S>>),
        }

        // The cyclic bound below is WRAPPED, so structural strips it and asserts the container
        // forwards the trait — deliberately, see `wrapped_bound_not_forwarded.rs`. Supply the
        // forwarding impl, exactly as syan's `Box<T>: Parse<Atom>` does.
        impl<T: Tr> Tr for Box<T> {
            fn total(&self) -> usize {
                (**self).total()
            }
        }

        impl<S: Clone + Default> Tr for Expr<S>
        where
            Box<Expr<S>>: Tr,
        {
            fn total(&self) -> usize {
                match self {
                    Expr::Leaf(_) => 1,
                    Expr::Nest(inner) => inner.total() + 1,
                }
            }
        }
    }

    #[test]
    fn compiles_and_runs() {
        use ast::{Expr, Tr};
        let e = Expr::Nest(Box::new(Expr::Nest(Box::new(Expr::Leaf(0u8)))));
        assert_eq!(e.total(), 3);
    }
}

/// The forwarding assertion is skipped when the stripped bound still mentions an impl generic —
/// including one that appears **only in an associated-type binding**.
///
/// `Box<Expr<X>>: Tr<Assoc = X>` is a wrapped cyclic bound, so stripping it would normally emit a
/// `const _` forwarding assertion. That assertion is generic over the container's element only, so a
/// parameter mentioned elsewhere in the predicate is not in scope inside it. The guard for exactly
/// this existed, but `path_mentions` inspected only `GenericArgument::Type` and never
/// `GenericArgument::AssocType` — so `Assoc = X` did not register and the assertion was emitted
/// referring to an unbound `X` (`E0412: cannot find type X in this scope`).
///
/// This is not an exotic shape: an associated-type constraint on a cyclic bound is what syan's
/// `Spanned` derive always produces (`Spanned<Span = __Syan_Span>`).
mod assoc_binding_keeps_the_guard_honest {
    use super::*;

    #[decycle(structural)]
    mod ast {
        #[decycle]
        pub trait Tr {
            type Assoc;
            fn get(&self) -> Self::Assoc;
        }

        pub enum Expr<X> {
            Leaf(X),
            Nest(Box<Expr<X>>),
        }

        impl<X: Clone + Default> Tr for Expr<X>
        where
            Box<Expr<X>>: Tr<Assoc = X>,
        {
            type Assoc = X;

            fn get(&self) -> X {
                match self {
                    Expr::Leaf(x) => x.clone(),
                    Expr::Nest(inner) => inner.get(),
                }
            }
        }
    }

    #[test]
    fn compiles_and_runs() {
        use ast::{Expr, Tr};
        let e: Expr<u8> = Expr::Nest(Box::new(Expr::Leaf(7)));
        assert_eq!(e.get(), 7);
    }
}
