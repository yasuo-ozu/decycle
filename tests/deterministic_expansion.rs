//! The same module always expands to the same tokens.
//!
//! An expansion that depends on a `HashMap`'s iteration order changes from one build to the next
//! (`std`'s hasher is seeded per process, and per map within one), which defeats reproducible builds
//! and incremental compilation. The ranked engine used to emit its per-trait items in the order of
//! one; every module here has several traits and several types, so that order would show.

use syn::parse_quote;

// The ranked engine reports through `proc_macro_error`, so it can only run inside a proc macro:
// `expand_repeatedly` expands the module eight times in one process and asserts that every run is
// the same, then emits it.
#[expand_bridge::expand_repeatedly]
mod ranked {
    #[decycle]
    pub trait Base {
        fn base(&self) -> u32;
    }

    #[decycle]
    pub trait Eval: Base {
        fn eval(&self) -> u32;
    }

    #[decycle]
    pub trait Size {
        fn size(&self) -> u32;
    }

    #[decycle]
    pub trait Show {
        fn show(&self) -> u32;
    }

    pub struct Expr(pub Option<Box<Term>>);
    pub struct Term(pub Option<Box<Stmt>>);
    pub struct Stmt(pub Option<Box<Expr>>);

    impl Base for Expr
    where
        Term: Base,
    {
        fn base(&self) -> u32 {
            self.0.as_ref().map_or(1, |t| t.base() + 1)
        }
    }

    impl Base for Term
    where
        Stmt: Base,
    {
        fn base(&self) -> u32 {
            self.0.as_ref().map_or(1, |s| s.base() + 1)
        }
    }

    impl Base for Stmt
    where
        Expr: Base,
    {
        fn base(&self) -> u32 {
            self.0.as_ref().map_or(1, |e| e.base() + 1)
        }
    }

    impl Eval for Expr
    where
        Term: Eval + Size,
    {
        fn eval(&self) -> u32 {
            self.base() + self.0.as_ref().map_or(0, |t| t.eval() + t.size())
        }
    }

    impl Eval for Term
    where
        Stmt: Eval,
    {
        fn eval(&self) -> u32 {
            self.0.as_ref().map_or(0, |s| s.eval())
        }
    }

    impl Eval for Stmt
    where
        Expr: Eval,
    {
        fn eval(&self) -> u32 {
            self.0.as_ref().map_or(0, |e| e.eval())
        }
    }

    impl Size for Expr
    where
        Term: Size,
    {
        fn size(&self) -> u32 {
            self.0.as_ref().map_or(1, |t| t.size() + 1)
        }
    }

    impl Size for Term
    where
        Stmt: Show,
    {
        fn size(&self) -> u32 {
            self.0.as_ref().map_or(1, |s| s.show() + 1)
        }
    }

    impl Size for Stmt
    where
        Expr: Size,
    {
        fn size(&self) -> u32 {
            self.0.as_ref().map_or(1, |e| e.size() + 1)
        }
    }

    impl Show for Stmt
    where
        Expr: Size,
    {
        fn show(&self) -> u32 {
            self.0.as_ref().map_or(1, |e| e.size())
        }
    }
}

#[test]
fn ranked_expansion_is_the_same_every_time_and_works() {
    use ranked::{Base, Eval, Expr, Size, Stmt, Term};
    let mut e = Expr(None);
    for _ in 0..20 {
        e = Expr(Some(Box::new(Term(Some(Box::new(Stmt(Some(Box::new(
            e,
        )))))))));
    }
    assert_eq!(e.base(), 61);
    assert_eq!(e.size(), 41);
    assert!(e.eval() > 0);
}

fn module() -> syn::ItemMod {
    parse_quote! {
        mod m {
            #[decycle] pub trait Base { fn base(&self) -> u32; }
            #[decycle] pub trait Eval: Base { fn eval(&self) -> u32; }
            #[decycle] pub trait Size { fn size(&self) -> u32; }
            pub struct Expr(pub Option<Box<Term>>);
            pub struct Term(pub Option<Box<Stmt>>);
            pub struct Stmt(pub Option<Box<Expr>>);
            impl Base for Expr where Term: Base { fn base(&self) -> u32 { 1 } }
            impl Base for Term where Stmt: Base { fn base(&self) -> u32 { 1 } }
            impl Base for Stmt where Expr: Base { fn base(&self) -> u32 { 1 } }
            impl Eval for Expr where Term: Eval + Size { fn eval(&self) -> u32 { 1 } }
            impl Eval for Term where Stmt: Eval { fn eval(&self) -> u32 { 1 } }
            impl Eval for Stmt where Expr: Eval { fn eval(&self) -> u32 { 1 } }
            impl Size for Expr where Term: Size { fn size(&self) -> u32 { 1 } }
            impl Size for Term where Stmt: Size { fn size(&self) -> u32 { 1 } }
            impl Size for Stmt where Expr: Size { fn size(&self) -> u32 { 1 } }
        }
    }
}

#[test]
fn structural_expansion_is_the_same_every_time() {
    let decycle: syn::Path = parse_quote!(::decycle);
    let module_path: syn::Path = parse_quote!(crate::m);
    let first = decycle::structural::process_module(module(), &decycle).to_string();
    let first_at =
        decycle::structural::process_module_at(module(), &module_path, &decycle).to_string();
    assert!(
        first.contains("__DecycleBody"),
        "sanity: the module is unrolled"
    );
    for _ in 0..8 {
        assert_eq!(
            decycle::structural::process_module(module(), &decycle).to_string(),
            first
        );
        assert_eq!(
            decycle::structural::process_module_at(module(), &module_path, &decycle).to_string(),
            first_at
        );
    }
}
