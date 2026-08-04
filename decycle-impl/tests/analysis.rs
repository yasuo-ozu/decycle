//! `analysis::analyze_module` — the code-free obligation graph shared by both engines.

use decycle_impl::analysis::{analyze_module, EdgeKind};
use decycle_impl::safegraph::graph::Graph;
use syn::parse_quote;

/// `(from, kind, to)` triples, sorted, so assertions do not depend on insertion order.
fn edges(module: &syn::ItemMod) -> Vec<(String, EdgeKind, String)> {
    let g = analyze_module(module, &parse_quote!(::decycle));
    // Indices only exist inside a scope for a `Vec`-backed graph.
    let mut out = g.scope(|ctx| {
        ctx.edge_indices()
            .map(|e| {
                let [a, b] = ctx.endpoints(e);
                (ctx.node(a).to_string(), *ctx.edge(e), ctx.node(b).to_string())
            })
            .collect::<Vec<_>>()
    });
    out.sort();
    out
}

fn nodes(module: &syn::ItemMod) -> Vec<String> {
    let g = analyze_module(module, &parse_quote!(::decycle));
    let mut out: Vec<String> = g.nodes().map(|n| n.to_string()).collect();
    out.sort();
    out
}

#[test]
fn direct_and_peeled() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub enum Expr { A }
            pub enum Stmt { B }
            // Wrapped: the head is `Vec`, so `Stmt` is only reachable inside the arguments.
            impl Tr for Expr where Vec<Box<Stmt>>: Tr { fn f(&self) {} }
            // Direct: the bound names `Expr` outright.
            impl Tr for Stmt where Expr: Tr { fn f(&self) {} }
        }
    };
    assert_eq!(nodes(&m), vec!["Expr", "Stmt"]);
    assert_eq!(
        edges(&m),
        vec![
            ("Expr".into(), EdgeKind::Peeled, "Stmt".into()),
            ("Stmt".into(), EdgeKind::Direct, "Expr".into()),
        ]
    );
}

#[test]
fn only_routed_traits_and_bare_spellings_count() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub trait Other { fn g(&self); }   // NOT routed
            pub enum Expr { A }
            pub enum Stmt { B }
            pub enum Leaf { C }
            impl Tr for Expr
            where
                Stmt: Tr,                 // routed + bare  -> edge
                Leaf: Other,              // not routed     -> no edge
                Stmt: ::other::Tr,        // qualified      -> the documented opt-out, no edge
            { fn f(&self) {} }
            impl Tr for Stmt { fn f(&self) {} }
            impl Other for Leaf { fn g(&self) {} }   // Leaf is not a participant at all
        }
    };
    assert_eq!(
        nodes(&m),
        vec!["Expr", "Stmt"],
        "Leaf implements no routed trait"
    );
    assert_eq!(
        edges(&m),
        vec![("Expr".into(), EdgeKind::Direct, "Stmt".into())]
    );
}

#[test]
fn self_and_impl_params_are_not_edges() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct A<T>(T);
            impl<T> Tr for A<T> where Self: Tr, T: Tr { fn f(&self) {} }
        }
    };
    assert_eq!(nodes(&m), vec!["A"]);
    assert!(
        edges(&m).is_empty(),
        "neither endpoint names another participant"
    );
}

#[test]
fn one_edge_per_pair_even_with_several_routed_traits() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle] pub trait P { fn p(&self); }
            #[decycle] pub trait U { fn u(&self); }
            pub enum Expr { A }
            pub enum Stmt { B }
            impl P for Expr where Stmt: P { fn p(&self) {} }
            impl U for Expr where Stmt: U { fn u(&self) {} }
            impl P for Stmt { fn p(&self) {} }
            impl U for Stmt { fn u(&self) {} }
        }
    };
    assert_eq!(
        edges(&m),
        vec![("Expr".into(), EdgeKind::Direct, "Stmt".into())]
    );
}

#[test]
fn decycle_use_routes_a_foreign_trait() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            use ::somewhere::Tr;
            pub enum Expr { A }
            pub enum Stmt { B }
            impl Tr for Expr where Box<Stmt>: Tr { fn f(&self) {} }
            impl Tr for Stmt { fn f(&self) {} }
        }
    };
    assert_eq!(nodes(&m), vec!["Expr", "Stmt"]);
    assert_eq!(
        edges(&m),
        vec![("Expr".into(), EdgeKind::Peeled, "Stmt".into())]
    );
}

#[test]
fn empty_and_traitless_modules_are_empty_graphs() {
    let bodyless: syn::ItemMod = parse_quote!(mod ast;);
    assert!(nodes(&bodyless).is_empty());

    let no_traits: syn::ItemMod = parse_quote! {
        mod ast { pub enum Expr { A } impl Expr { fn f(&self) {} } }
    };
    assert!(nodes(&no_traits).is_empty());
}

// ---------------------------------------------------------------------------------------------
// The graph-taking engine entry points
// ---------------------------------------------------------------------------------------------

fn cycle_module() -> syn::ItemMod {
    parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self) -> i64; }
            pub enum Expr { Lit(i64), Nest(Box<Stmt>) }
            pub enum Stmt { E(Box<Expr>) }
            impl Tr for Expr where Box<Stmt>: Tr {
                fn f(&self) -> i64 { match self { Expr::Lit(n) => *n, Expr::Nest(s) => s.f() } }
            }
            impl Tr for Stmt where Box<Expr>: Tr {
                fn f(&self) -> i64 { match self { Stmt::E(e) => e.f() } }
            }
        }
    }
}

// NOTE: the RANKED entry points cannot be called from a plain integration test — they use
// `proc_macro_error`'s `abort!`, which panics outside a proc-macro entry point. They are covered
// end-to-end by `tests/graph_bridge` + `tests/graph_entry.rs` instead. Structural returns
// `syn::Result` and so can be called directly.

/// Feeding back exactly what `analyze_module` produced must reproduce the default expansion.
#[test]
fn structural_round_trip_matches_the_derived_run() {
    let m = cycle_module();
    let p: syn::Path = parse_quote!(::decycle);
    let g = analyze_module(&m, &p);

    let derived = decycle_impl::structural::process_module(m.clone(), &p).to_string();
    let supplied = decycle_impl::structural::process_module_with_graph(m, &g, &p).to_string();
    assert_eq!(derived, supplied);
}

/// An empty node set means "nothing participates" — the engine must leave the impls alone.
#[test]
fn an_empty_graph_disables_structural_unrolling() {
    let m = cycle_module();
    let p: syn::Path = parse_quote!(::decycle);
    let empty = analyze_module(&parse_quote!(mod nothing {}), &p);

    let full = decycle_impl::structural::process_module(m.clone(), &p).to_string();
    let none = decycle_impl::structural::process_module_with_graph(m, &empty, &p).to_string();
    assert_ne!(full, none, "an empty participant set must change the output");
    assert!(
        !none.contains("Term"),
        "no participants => no terminator type is generated"
    );
    assert!(full.contains("Term"), "sanity: the derived run does generate one");
}
