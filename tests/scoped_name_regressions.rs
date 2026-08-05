//! Regressions for identity-by-bare-ident defects (2026-08-05 audit): every matcher that
//! answers "does this name mean the module-local cycle item?" must consult scope, not just
//! compare idents / last path segments.

#![allow(dead_code)] // codegen fixtures: variants are built, never read
use decycle::decycle;

/// Defect 1: an impl's own generic parameter that SHADOWS a cycle-head name must not be
/// rewritten to the module-level alias of the struct. `AliasHeads` used to re-spell the
/// first segment of any non-rooted path matching a cycle-head ident, turning every use of
/// the PARAMETER `Stmt` below into the STRUCT `Stmt` (E0207 + E0277 + ranked overflow).
mod shadowed_generic_param {
    use super::decycle;

    #[decycle]
    mod m {
        #[decycle]
        pub trait Tr {
            fn f(&self) -> usize;
        }
        pub struct Stmt(pub Option<Box<Wrap<i64>>>);
        pub struct Wrap<T>(pub T);
        impl Tr for Stmt
        where
            Wrap<i64>: Tr,
        {
            fn f(&self) -> usize {
                self.0.as_ref().map(|w| w.f()).unwrap_or(1)
            }
        }
        // The parameter deliberately shadows the cycle head `Stmt`.
        impl<Stmt> Tr for Wrap<Stmt>
        where
            Stmt: Copy,
        {
            fn f(&self) -> usize {
                2
            }
        }
    }

    #[test]
    fn shadowing_param_is_not_aliased_to_the_struct() {
        use m::Tr;
        assert_eq!(m::Stmt(None).f(), 1);
        assert_eq!(m::Stmt(Some(Box::new(m::Wrap(0)))).f(), 2);
        assert_eq!(m::Wrap(0i64).f(), 2);
    }
}

/// Defect 4: a cycle-head name inside the token stream of a std macro invocation
/// (`matches!(self, Stmt::Leaf)`) is re-emitted under the helper-module double glob, where
/// the bare name is ambiguous whenever an outer scope also defines it (E0659). The alias
/// pass must reach into (allowlisted, expression-position) macro tokens too.
mod cycle_head_in_macro_tokens {
    use super::decycle;

    // Same name at the enclosing scope — this is what makes the double glob ambiguous.
    #[allow(dead_code)]
    pub struct Stmt;

    #[decycle]
    mod m {
        #[decycle]
        pub trait Tr {
            fn f(&self) -> usize;
        }
        pub enum Stmt {
            Leaf,
            Node(Box<Expr>),
        }
        pub struct Expr(pub Option<Box<Stmt>>);
        impl Tr for Stmt
        where
            Expr: Tr,
        {
            fn f(&self) -> usize {
                if matches!(self, Stmt::Leaf) {
                    1
                } else {
                    0
                }
            }
        }
        impl Tr for Expr
        where
            Stmt: Tr,
        {
            fn f(&self) -> usize {
                self.0.as_ref().map(|s| s.f()).unwrap_or(2)
            }
        }
    }

    #[test]
    fn macro_tokens_resolve_the_local_cycle_head() {
        use m::Tr;
        assert_eq!(m::Stmt::Leaf.f(), 1);
        assert_eq!(m::Stmt::Node(Box::new(m::Expr(None))).f(), 0);
        assert_eq!(m::Expr(Some(Box::new(m::Stmt::Leaf))).f(), 1);
        assert_eq!(m::Expr(None).f(), 2);
    }
}

/// Defect 4 guard: a macro that treats idents as DATA must not have its tokens rewritten —
/// `stringify!` output has to keep the spelling the user wrote even when the ident names a
/// cycle head (no outer shadow here, so the bare name resolves fine through the glob).
mod stringify_is_left_verbatim {
    use super::decycle;

    #[decycle]
    mod m {
        #[decycle]
        pub trait Tr {
            fn name(&self) -> &'static str;
        }
        pub struct Stmt(pub Option<Box<Expr>>);
        pub struct Expr;
        impl Tr for Stmt
        where
            Expr: Tr,
        {
            fn name(&self) -> &'static str {
                stringify!(Stmt)
            }
        }
        impl Tr for Expr
        where
            Stmt: Tr,
        {
            fn name(&self) -> &'static str {
                // Nested non-allowlisted macro inside an allowlisted one: the inner
                // `stringify!` tokens must stay verbatim too.
                assert_eq!(stringify!(Stmt), "Stmt");
                stringify!(Expr)
            }
        }
    }

    #[test]
    fn stringify_keeps_the_user_spelling() {
        use m::Tr;
        assert_eq!(m::Stmt(None).name(), "Stmt");
        assert_eq!(m::Expr.name(), "Expr");
    }
}
