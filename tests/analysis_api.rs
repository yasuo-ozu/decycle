//! The obligation-graph API is reachable from every documented path.

use decycle::safegraph::graph::Graph;
use syn::parse_quote;

#[test]
fn reachable_from_all_three_paths() {
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

    // The shared module, and the two engine namespaces that re-export it.
    let a = decycle::analysis::analyze_module(&m, &p);
    let b = decycle::ranked::analyze_module(&m, &p);
    let c = decycle::structural::analyze_module(&m, &p);

    for g in [&a, &b, &c] {
        let mut ns: Vec<String> = g.nodes().map(|n| n.to_string()).collect();
        ns.sort();
        assert_eq!(ns, vec!["Expr", "Stmt"]);
        assert_eq!(g.len_edge(), 2);
    }

    // `EdgeKind` is the same type through every path.
    let kinds: Vec<decycle::analysis::EdgeKind> = a.edges().copied().collect();
    assert!(kinds.contains(&decycle::ranked::EdgeKind::Peeled));
    assert!(kinds.contains(&decycle::structural::EdgeKind::Direct));
}
