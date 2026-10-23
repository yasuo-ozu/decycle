//! Unit tests of the obligation graph (`analyze_module`, `cyclic_subgraph`).
//!
//! Regressions kept from the audits of the former public API:
//!
//! 1. A generic parameter of an impl that shadows a participant's name must not fabricate an
//!    edge: a bound targeting one of the impl's own type parameters is "not represented", and the
//!    engine (`remove_cyclic_bounds` / `cyclic_where_bounds`) already consults the impl's param
//!    idents.
//! 2. A raw-identifier participant (`struct r#loop`) must not panic `analyze_module` — the
//!    node idents are cloned from the source, never re-created via `Ident::new`.
//! 3. A bound wrapped in a container that is ITSELF a participant (`Wrap<Stmt>: Tr`, `Wrap`
//!    implementing the trait too) must report both endpoints. Emitting only the head edge made
//!    `cyclic_subgraph` answer "no participants" for a fully cyclic module.
//! 4. A bound whose target is a FOREIGN type sharing its last path segment with a participant
//!    (`::other::Stmt: Tr`) is no edge at all — the same bare-or-`self::` rule the engine's own
//!    `peel::cycle_types_within` applies.

use super::{analyze_module, cyclic_subgraph, EdgeKind};
use proc_macro2::Ident;
use safegraph::graph::Graph;
use safegraph::VecGraph;
use syn::parse_quote;

/// `(from, kind, to)` triples, sorted, so assertions do not depend on insertion order.
fn edges_of(g: &VecGraph<Ident, EdgeKind>) -> Vec<(String, EdgeKind, String)> {
    let (nodes, edges) = crate::graph::decompose(g);
    let mut out: Vec<_> = edges
        .into_iter()
        .map(|(a, b, kind)| (nodes[a].to_string(), kind, nodes[b].to_string()))
        .collect();
    out.sort();
    out
}

fn edges(module: &syn::ItemMod) -> Vec<(String, EdgeKind, String)> {
    edges_of(&analyze_module(module, &parse_quote!(::decycle)))
}

fn sorted_nodes(g: &VecGraph<Ident, EdgeKind>) -> Vec<String> {
    let mut v: Vec<String> = g.nodes().map(|n| n.to_string()).collect();
    v.sort();
    v
}

fn nodes(module: &syn::ItemMod) -> Vec<String> {
    sorted_nodes(&analyze_module(module, &parse_quote!(::decycle)))
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
    let bodyless: syn::ItemMod = parse_quote!(
        mod ast;
    );
    assert!(nodes(&bodyless).is_empty());

    let no_traits: syn::ItemMod = parse_quote! {
        mod ast { pub enum Expr { A } impl Expr { fn f(&self) {} } }
    };
    assert!(nodes(&no_traits).is_empty());
}

/// The qualified spelling is the "not an edge" opt-out, and that cuts both ways: a module whose
/// bounds are *all* qualified has no participants at all.
///
/// So `analyze_module` cannot be used to *discover* a caller's cycles: it reads a classification
/// that the caller has already expressed in how it spells each path.
#[test]
fn a_fully_qualified_module_has_no_participants() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            use ::syan::decycle_traits::Parse;
            pub enum Expr<S> { Lit, Nest(Box<Stmt<S>>) }
            pub enum Stmt<S> { E(Box<Expr<S>>) }
            impl<S, A> ::syan::parse::parse::Parse<A> for Expr<S>
            where Box<Stmt<S>>: ::syan::parse::parse::Parse<A> {}
            impl<S, A> ::syan::parse::parse::Parse<A> for Stmt<S>
            where Box<Expr<S>>: ::syan::parse::parse::Parse<A> {}
        }
    };
    assert!(
        nodes(&m).is_empty(),
        "a qualified impl is deliberately not adopted, so it cannot make its self type a participant"
    );
}

#[test]
fn cyclic_subgraph_keeps_only_recursive_nodes() {
    // Expr <-> Stmt is a real cycle; Leaf is pointed at by Expr but points nowhere.
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle] pub trait Tr { fn f(&self); }
            pub enum Expr { A } pub enum Stmt { B } pub enum Leaf { C }
            impl Tr for Expr where Box<Stmt>: Tr, Leaf: Tr { fn f(&self) {} }
            impl Tr for Stmt where Expr: Tr { fn f(&self) {} }
            impl Tr for Leaf { fn f(&self) {} }
        }
    };
    let full = analyze_module(&m, &parse_quote!(::decycle));
    assert_eq!(sorted_nodes(&full), vec!["Expr", "Leaf", "Stmt"]);

    let cyc = cyclic_subgraph(&full);
    assert_eq!(
        sorted_nodes(&cyc),
        vec!["Expr", "Stmt"],
        "Leaf is referenced but not recursive"
    );
    // The Expr -> Leaf edge goes with it; Expr <-> Stmt survives.
    assert_eq!(cyc.len_edge(), 2);
}

#[test]
fn a_self_edge_is_a_cycle_of_one() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle] pub trait Tr { fn f(&self); }
            pub enum Expr { A }
            impl Tr for Expr where Box<Expr>: Tr { fn f(&self) {} }
        }
    };
    let cyc = cyclic_subgraph(&analyze_module(&m, &parse_quote!(::decycle)));
    assert_eq!(sorted_nodes(&cyc), vec!["Expr"]);
}

#[test]
fn a_lone_referenced_node_is_not_a_cycle() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle] pub trait Tr { fn f(&self); }
            pub enum A { X } pub enum B { Y }
            impl Tr for A where B: Tr { fn f(&self) {} }   // A -> B, no way back
            impl Tr for B { fn f(&self) {} }
        }
    };
    let cyc = cyclic_subgraph(&analyze_module(&m, &parse_quote!(::decycle)));
    assert!(sorted_nodes(&cyc).is_empty(), "a DAG has no cyclic nodes");
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
    use proc_macro2::Span;

    // a0 <-> b is a real cycle; a1 (same NAME as a0) is an isolated bystander.
    let g: VecGraph<Ident, EdgeKind> = crate::graph::compose(
        ["A", "B", "A"].map(|name| Ident::new(name, Span::call_site())),
        [(0, 1, EdgeKind::Direct), (1, 0, EdgeKind::Direct)],
    );

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

/// The structural engine unrolls this cycle exactly where the graph says it is: both members get a
/// terminator, `Wrap` (which only forwards its own parameter) none — the module
/// `tests/nested_participant_cycle.rs` compiles and runs.
#[test]
fn the_structural_run_emits_terminators_for_a_wrapped_cycle() {
    let p: syn::Path = parse_quote!(::decycle);
    let out = crate::structural::process_module(wrapped_cycle_module(), &p).to_string();
    assert!(
        out.contains("__ExprTerm_") && out.contains("__StmtTerm_"),
        "both cycle members must get a terminator: {out}"
    );
    assert!(
        !out.contains("__WrapTerm_"),
        "Wrap is not on the cycle, so it needs none"
    );
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

/// A self type spelled by a path names the item itself: like a path-spelled trait, it is no node of
/// the module's graph, which stays the engines' own participant set. `self::` is the bare name.
#[test]
fn self_types_are_read_by_their_bare_name() {
    let m: syn::ItemMod = parse_quote! {
        mod m {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub struct Expr;
            pub struct Term;
            pub struct Stmt;
            impl Tr for Expr where Term: Tr { fn f(&self) {} }
            impl Tr for self::Term where Expr: Tr { fn f(&self) {} }
            impl Tr for crate::m::Stmt where Expr: Tr { fn f(&self) {} }
            impl Tr for ::other::Leaf { fn f(&self) {} }
        }
    };
    let g = analyze_module(&m, &parse_quote!(::decycle));
    assert_eq!(sorted_nodes(&g), vec!["Expr", "Term"]);
    assert_eq!(sorted_nodes(&cyclic_subgraph(&g)), vec!["Expr", "Term"]);
}

/// Without a graph, an impl of a path-spelled self type beside a member of the same name is no
/// member: the structural engine re-emits it as written, next to the member's generated impls.
#[test]
fn structural_path_spelled_self_type_is_emitted_as_written() {
    let m: syn::ItemMod = parse_quote! {
        mod m {
            #[decycle] pub trait Loop { fn step(&self) -> u32; }
            pub struct A;
            pub struct B;
            impl Loop for crate::other::B { fn step(&self) -> u32 { 7 } }
            impl Loop for A where B: Loop { fn step(&self) -> u32 { 1 } }
            impl Loop for B where A: Loop { fn step(&self) -> u32 { 2 } }
        }
    };
    let out = crate::structural::process_module(m, &parse_quote!(::decycle)).to_string();
    assert!(out.contains("__BTerm_"), "B is still a member: {out}");
    assert!(
        out.contains("impl Loop for crate :: other :: B"),
        "the foreign impl is re-emitted as written: {out}"
    );
}

/// The graph is built in a deterministic order (nodes sorted, edges in item order), so two runs over
/// one module agree to the index.
#[test]
fn the_graph_is_the_same_every_time() {
    let m: syn::ItemMod = parse_quote! {
        mod m {
            #[decycle] pub trait Base { fn base(&self) -> u32; }
            #[decycle] pub trait Size { fn size(&self) -> u32; }
            pub struct Expr; pub struct Term; pub struct Stmt;
            impl Base for Expr where Term: Base { fn base(&self) -> u32 { 1 } }
            impl Base for Term where Stmt: Base { fn base(&self) -> u32 { 1 } }
            impl Base for Stmt where Expr: Base { fn base(&self) -> u32 { 1 } }
            impl Size for Expr where Vec<Term>: Size { fn size(&self) -> u32 { 1 } }
            impl Size for Term where Expr: Size { fn size(&self) -> u32 { 1 } }
        }
    };
    let render = || {
        let g = analyze_module(&m, &parse_quote!(::decycle));
        let nodes: Vec<String> = g.nodes().map(|n| n.to_string()).collect();
        let edges: Vec<String> = g.edges().map(|e| format!("{e:?}")).collect();
        (nodes, edges, edges_of(&g))
    };
    let first = render();
    assert_eq!(first.0, vec!["Expr", "Stmt", "Term"]);
    for _ in 0..8 {
        assert_eq!(render(), first);
    }
}

/// A leading `::` on one segment (`::Tr`) is a path like any other, the opt-out: an impl of `::Tr`
/// is no node, and a bound naming `::Tr` no edge.
#[test]
fn a_leading_colon_is_a_path() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle]
            pub trait Tr { fn f(&self); }
            pub enum Expr { A }
            pub enum Stmt { B }
            pub enum Leaf { C }
            impl Tr for Expr where Stmt: ::Tr, Leaf: Tr { fn f(&self) {} }
            impl Tr for Stmt where Expr: Tr { fn f(&self) {} }
            impl ::Tr for Leaf where Expr: Tr { fn f(&self) {} }
        }
    };
    assert_eq!(nodes(&m), vec!["Expr", "Stmt"], "`impl ::Tr for Leaf` is no node");
    assert_eq!(
        edges(&m),
        vec![("Stmt".into(), EdgeKind::Direct, "Expr".into())],
        "`Stmt: ::Tr` is no edge"
    );
}
