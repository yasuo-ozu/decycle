//! Regressions for the obligation-graph API (`analysis.rs`) — the 2026-08-05 front-door audit.
//!
//! 1. A generic parameter of an impl that shadows a participant's name must not fabricate an
//!    edge: the doc promises that a bound targeting one of the impl's own type parameters is
//!    "not represented", and the engine (`remove_cyclic_bounds` / `cyclic_where_bounds`)
//!    already consults the impl's param idents.
//! 2. A raw-identifier participant (`struct r#loop`) must not panic `analyze_module` — the
//!    node idents are cloned from the source, never re-created via `Ident::new`.

use decycle_impl::analysis::{analyze_module, cyclic_subgraph, EdgeKind};
use decycle_impl::safegraph::graph::Graph;
use syn::parse_quote;

/// `(from, kind, to)` triples, sorted, so assertions do not depend on insertion order.
fn edges_of(g: &decycle_impl::safegraph::VecGraph<proc_macro2::Ident, EdgeKind>) -> Vec<(String, EdgeKind, String)> {
    let mut out = g.scope(|ctx| {
        ctx.edge_indices()
            .map(|e| {
                let [a, b] = ctx.endpoints(e);
                (
                    ctx.node(a).to_string(),
                    *ctx.edge(e),
                    ctx.node(b).to_string(),
                )
            })
            .collect::<Vec<_>>()
    });
    out.sort();
    out
}

fn edges(module: &syn::ItemMod) -> Vec<(String, EdgeKind, String)> {
    edges_of(&analyze_module(module, &parse_quote!(::decycle)))
}

/// Defect 1: `impl<Stmt> Tr for Expr where Stmt: Tr` — the bound targets the impl's OWN type
/// parameter `Stmt`, which merely shadows the participant of the same name. No edge, no cycle.
#[test]
fn own_type_param_shadowing_a_participant_is_not_an_edge() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct Stmt;
            pub struct Expr;
            impl Tr for Stmt { fn f(&self) {} }
            impl<Stmt> Tr for Expr where Stmt: Tr { fn f(&self) {} }
        }
    };
    assert_eq!(
        edges(&m),
        Vec::<(String, EdgeKind, String)>::new(),
        "a bound on the impl's own type parameter is not a graph edge"
    );
    // And therefore no fabricated cycle either.
    let cyc = cyclic_subgraph(&analyze_module(&m, &parse_quote!(::decycle)));
    let n: Vec<String> = cyc.nodes().map(|n| n.to_string()).collect();
    assert!(n.is_empty(), "no cycle: {n:?}");
}

/// Defect 1, control: the same shape WITHOUT the shadowing parameter is a real edge — the skip
/// must key on the impl's own generics, not on the name.
#[test]
fn unshadowed_bound_of_the_same_shape_still_counts() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct Stmt;
            pub struct Expr;
            impl Tr for Stmt where Expr: Tr { fn f(&self) {} }
            impl Tr for Expr where Stmt: Tr { fn f(&self) {} }
        }
    };
    assert_eq!(
        edges(&m),
        vec![
            ("Expr".into(), EdgeKind::Direct, "Stmt".into()),
            ("Stmt".into(), EdgeKind::Direct, "Expr".into()),
        ]
    );
}

/// Defect 1, sibling promise: a bound targeting `Self` is likewise not represented.
#[test]
fn self_bound_is_not_an_edge() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct Expr;
            impl Tr for Expr where Self: Tr { fn f(&self) {} }
        }
    };
    assert_eq!(edges(&m), Vec::<(String, EdgeKind, String)>::new());
}

/// Defect 2: a raw-identifier participant must round-trip without panicking, keeping its raw
/// spelling in the returned graph.
#[test]
fn raw_ident_participant_does_not_panic() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct r#loop;
            pub struct Expr;
            impl Tr for r#loop where Expr: Tr { fn f(&self) {} }
            impl Tr for Expr where r#loop: Tr { fn f(&self) {} }
        }
    };
    let g = analyze_module(&m, &parse_quote!(::decycle));
    let mut nodes: Vec<String> = g.nodes().map(|n| n.to_string()).collect();
    nodes.sort();
    assert_eq!(nodes, vec!["Expr", "r#loop"]);
    assert_eq!(
        edges_of(&g),
        vec![
            ("Expr".into(), EdgeKind::Direct, "r#loop".into()),
            ("r#loop".into(), EdgeKind::Direct, "Expr".into()),
        ]
    );
    // The cycle machinery must survive the raw ident too (compose/decompose round-trip).
    let cyc = cyclic_subgraph(&g);
    let mut nodes: Vec<String> = cyc.nodes().map(|n| n.to_string()).collect();
    nodes.sort();
    assert_eq!(nodes, vec!["Expr", "r#loop"]);
}

/// A caller-built graph may (unlike anything `analyze_module` returns) contain two nodes with
/// the same name. `cyclic_subgraph` used to key nodes by `Ident::to_string()` on the way out and
/// back in, conflating them: the cycle's edges were reattached to the LAST same-named node and
/// the bystander survived into the result. Nodes are now tracked by position.
#[test]
fn cyclic_subgraph_keeps_same_named_nodes_distinct() {
    use decycle_impl::safegraph::VecGraph;
    use proc_macro2::{Ident, Span};

    let mut g: VecGraph<Ident, EdgeKind> = VecGraph::default();
    g.scope_mut(|mut ctx| {
        // a0 <-> b is a real cycle; a1 (same NAME as a0) is an isolated bystander.
        let a0 = ctx.insert_node(Ident::new("A", Span::call_site())).unwrap();
        let b = ctx.insert_node(Ident::new("B", Span::call_site())).unwrap();
        let _a1 = ctx.insert_node(Ident::new("A", Span::call_site())).unwrap();
        ctx.insert_edge(EdgeKind::Direct, [a0, b]).unwrap();
        ctx.insert_edge(EdgeKind::Direct, [b, a0]).unwrap();
    });

    let cyc = cyclic_subgraph(&g);
    let mut nodes: Vec<String> = cyc.nodes().map(|n| n.to_string()).collect();
    nodes.sort();
    assert_eq!(nodes, vec!["A", "B"], "only the cycling A survives, once");
    assert_eq!(
        edges_of(&cyc),
        vec![
            ("A".into(), EdgeKind::Direct, "B".into()),
            ("B".into(), EdgeKind::Direct, "A".into()),
        ]
    );
}

/// `analysis.rs:72` used to `.expect()` on an empty `decycle` path. An empty path names no
/// crate, so nothing can be routed: the empty graph, not a panic.
#[test]
fn empty_decycle_path_yields_empty_graph_not_panic() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct Expr;
            impl Tr for Expr { fn f(&self) {} }
        }
    };
    let empty = syn::Path {
        leading_colon: None,
        segments: syn::punctuated::Punctuated::new(),
    };
    let g = analyze_module(&m, &empty);
    assert_eq!(g.nodes().count(), 0);
}
