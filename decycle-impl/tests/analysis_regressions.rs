//! Regressions for the obligation-graph API (`analysis.rs`) — the 2026-08-05 front-door audit.
//!
//! 1. A generic parameter of an impl that shadows a participant's name must not fabricate an
//!    edge: the doc promises that a bound targeting one of the impl's own type parameters is
//!    "not represented", and the engine (`remove_cyclic_bounds` / `cyclic_where_bounds`)
//!    already consults the impl's param idents.
//! 2. A raw-identifier participant (`struct r#loop`) must not panic `analyze_module` — the
//!    node idents are cloned from the source, never re-created via `Ident::new`.
//!
//! …and the 2026-08-14 pass:
//!
//! 3. A bound wrapped in a container that is ITSELF a participant (`Wrap<Stmt>: Tr`, `Wrap`
//!    implementing the trait too) must report both endpoints. Emitting only the head edge made
//!    `cyclic_subgraph` answer "no participants" for a fully cyclic module, and an engine fed that
//!    answer left the module un-decycled.
//! 4. A bound whose target is a FOREIGN type sharing its last path segment with a participant
//!    (`::other::Stmt: Tr`) is no edge at all — the same bare-or-`self::` rule the engine's own
//!    `peel::cycle_types_within` / `ranked::contract` apply.

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

// ---------------------------------------------------------------------------------------------
// Defect 3: a participating container must not swallow the participants nested inside it
// ---------------------------------------------------------------------------------------------

/// `Expr: Tr` needs `Wrap<Stmt>: Tr`, which needs `Stmt: Tr`, which needs `Wrap<Expr>: Tr`, which
/// needs `Expr: Tr` — a genuine cycle (E0275 without decycle), and one `#[decycle(structural)]`
/// breaks. Kept character-for-character identical to the module in
/// `tests/nested_participant_cycle.rs`, which compiles and runs it.
fn wrapped_cycle_module() -> syn::ItemMod {
    parse_quote! {
        mod structural {
            #[decycle]
            pub trait Tr {
                fn depth(&self) -> u32;
            }

            pub struct Wrap<T>(pub T);

            pub enum Expr {
                Lit,
                Nest(Box<Stmt>),
            }
            pub enum Stmt {
                E(Box<Expr>),
            }

            impl<T: Tr> Tr for Wrap<T> {
                fn depth(&self) -> u32 {
                    self.0.depth()
                }
            }

            impl Tr for Expr
            where
                Wrap<Stmt>: Tr,
            {
                fn depth(&self) -> u32 {
                    match self {
                        Expr::Lit => 0,
                        Expr::Nest(s) => s.depth() + 1,
                    }
                }
            }

            impl Tr for Stmt
            where
                Wrap<Expr>: Tr,
            {
                fn depth(&self) -> u32 {
                    match self {
                        Stmt::E(e) => e.depth(),
                    }
                }
            }
        }
    }
}

/// The head of `Wrap<Stmt>` is a participant, so the bound is a `Direct` edge to `Wrap` — and that
/// used to be ALL of it: the nested `Stmt` was dropped, both back-edges with it, and the graph came
/// out acyclic. Both endpoints are now reported.
#[test]
fn a_participating_container_does_not_swallow_its_nested_participants() {
    let m = wrapped_cycle_module();
    assert_eq!(
        edges(&m),
        vec![
            ("Expr".into(), EdgeKind::Direct, "Wrap".into()),
            ("Expr".into(), EdgeKind::Peeled, "Stmt".into()),
            ("Stmt".into(), EdgeKind::Direct, "Wrap".into()),
            ("Stmt".into(), EdgeKind::Peeled, "Expr".into()),
        ],
        "`Wrap<Stmt>: Tr` reaches Wrap directly AND Stmt inside it"
    );

    let cyc = cyclic_subgraph(&analyze_module(&m, &parse_quote!(::decycle)));
    let mut n: Vec<String> = cyc.nodes().map(|n| n.to_string()).collect();
    n.sort();
    assert_eq!(
        n,
        vec!["Expr", "Stmt"],
        "Expr <-> Stmt recurse through Wrap; Wrap itself only forwards its own parameter"
    );
    assert_eq!(
        edges_of(&cyc),
        vec![
            ("Expr".into(), EdgeKind::Peeled, "Stmt".into()),
            ("Stmt".into(), EdgeKind::Peeled, "Expr".into()),
        ]
    );
}

/// The documented workflow end to end: analyze -> `cyclic_subgraph` -> engine. The graph-fed
/// structural run must emit the very terminators the deriving run does — it used to be handed an
/// empty node set and emit nothing at all, silently leaving the module un-decycled (E0275 at the
/// use site).
#[test]
fn the_graph_fed_structural_run_emits_terminators_for_a_wrapped_cycle() {
    let p: syn::Path = parse_quote!(::decycle);
    let m = wrapped_cycle_module();
    let cyc = cyclic_subgraph(&analyze_module(&m, &p));

    let fed = decycle_impl::structural::process_module_with_graph(m.clone(), &cyc, &p).to_string();
    assert!(
        fed.contains("__ExprTerm_") && fed.contains("__StmtTerm_"),
        "both cycle members must get a terminator: {fed}"
    );
    assert!(
        !fed.contains("__WrapTerm_"),
        "Wrap is not on the cycle, so it needs none"
    );

    // Strongest available statement of "this works": the graph-fed expansion is byte-identical to
    // the deriving one, which `tests/nested_participant_cycle.rs` compiles and runs.
    let derived = decycle_impl::structural::process_module(m, &p).to_string();
    assert_eq!(derived, fed);
}

// ---------------------------------------------------------------------------------------------
// Defect 4: a foreign bound target is not an edge to the local type of the same name
// ---------------------------------------------------------------------------------------------

/// `::other::Stmt` / `crate::other::Stmt` are different items from the module's own `Stmt`. Matching
/// on the last path segment fabricated a cycle `{Expr, Stmt}` that does not exist.
#[test]
fn a_foreign_bound_target_is_not_an_edge_to_the_local_type_of_that_name() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct Expr;
            pub struct Stmt;
            impl Tr for Expr
            where
                ::other::Stmt: Tr,              // direct, foreign  -> no edge
                Box<crate::other::Stmt>: Tr,    // nested, foreign  -> no edge
            { fn f(&self) {} }
            impl Tr for Stmt where Expr: Tr { fn f(&self) {} }
        }
    };
    assert_eq!(
        edges(&m),
        vec![("Stmt".into(), EdgeKind::Direct, "Expr".into())],
        "only the local `Expr: Tr` bound is an edge"
    );
    let cyc = cyclic_subgraph(&analyze_module(&m, &parse_quote!(::decycle)));
    let n: Vec<String> = cyc.nodes().map(|n| n.to_string()).collect();
    assert!(n.is_empty(), "no cycle exists: {n:?}");
}

/// Control: the bare and the no-op `self::` spellings still name the local participant, and a local
/// participant nested inside a FOREIGN container is still reached.
#[test]
fn local_spellings_and_foreign_containers_still_reach_the_participant() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct Expr;
            pub struct Stmt;
            impl Tr for Expr where self::Stmt: Tr { fn f(&self) {} }
            impl Tr for Stmt where ::other::Wrap<Expr>: Tr { fn f(&self) {} }
        }
    };
    assert_eq!(
        edges(&m),
        vec![
            ("Expr".into(), EdgeKind::Direct, "Stmt".into()),
            ("Stmt".into(), EdgeKind::Peeled, "Expr".into()),
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
