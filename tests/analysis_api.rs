//! The obligation-graph API is reachable from the facade, and its result is accepted by both engines.
//!
//! There is exactly one path to the analysis — `decycle::analysis` — because the analysis is
//! engine-independent. It used to be re-exported under `ranked::` and `structural::` as well, which
//! only suggested the two might differ.

use decycle::safegraph::graph::Graph;
use syn::parse_quote;

#[test]
fn reachable_and_accepted_by_both_engines() {
    let m: syn::ItemMod = parse_quote! {
        mod ast {
            #[decycle] pub trait Tr { fn f(&self); }
            pub enum Expr { A }
            pub enum Stmt { B }
            impl Tr for Expr where Vec<Stmt>: Tr { fn f(&self) {} }
            impl Tr for Stmt where Expr: Tr { fn f(&self) {} }
        }
    };
    let p: syn::Path = parse_quote!(::decycle);

    let g = decycle::analysis::analyze_module(&m, &p);
    let mut ns: Vec<String> = g.nodes().map(|n| n.to_string()).collect();
    ns.sort();
    assert_eq!(ns, vec!["Expr", "Stmt"]);
    assert_eq!(g.len_edge(), 2);

    let kinds: Vec<decycle::analysis::EdgeKind> = g.edges().copied().collect();
    assert!(kinds.contains(&decycle::analysis::EdgeKind::Peeled));
    assert!(kinds.contains(&decycle::analysis::EdgeKind::Direct));

    // The same graph is accepted by an engine's graph-taking entry point. Only `structural` can be
    // called from here — `ranked` reports through `proc_macro_error`, which panics outside a
    // proc-macro entry point, so it is covered by `tests/graph_bridge` instead.
    //
    // The result has to be inspected, not discarded: on rejection this entry point *returns*
    // `quote! { #compile_error #module }` rather than panicking, so binding it to `_` made the
    // "accepted" half of this test's name unverifiable.
    let structural = decycle::structural::process_module_with_graph(m, &g, &p).to_string();
    assert!(
        !structural.contains("compile_error"),
        "structural rejected the analysed graph: {structural}"
    );
    assert!(
        structural.contains("__DecycleBody"),
        "structural accepted the graph but emitted no terminator machinery: {structural}"
    );
}
