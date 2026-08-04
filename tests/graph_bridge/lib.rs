//! Test-only bridge for the graph-taking engine entry points.
//!
//! `ranked::process_module_with_graph` cannot be exercised from a plain integration test: the ranked
//! engine reports through `proc_macro_error`, whose API panics outside a proc-macro entry point. So
//! the call is made here, inside a real `#[proc_macro_error]` function, exactly the way a wrapper
//! macro crate would make it.

use decycle_impl::analysis::analyze_module;
use proc_macro_error::proc_macro_error;
use decycle_impl::ranked::{process_module, process_module_with_graph};
use proc_macro::TokenStream;
use syn::{parse_macro_input, parse_quote, ItemMod};

/// Run the ranked engine over the given module, deriving the graph and feeding it straight back.
/// The output must be usable code — and identical to the plain `process_module` run.
#[proc_macro_attribute]
#[proc_macro_error]
pub fn decycle_via_graph(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let module = parse_macro_input!(input as ItemMod);
    let decycle: syn::Path = parse_quote!(::decycle);

    let graph = analyze_module(&module, &decycle);
    // Default options: the node set only. The impls in the test module are already spelled the
    // way the engine reads, so this must reproduce `process_module` exactly.
    let supplied = process_module_with_graph(
        module.clone(),
        &graph,
        /* emit_contracts */ false,
        &decycle,
        2,
        true,
    );

    // Same input through the deriving entry point: the round trip has to be exact.
    let derived = process_module(module, &decycle, 2, true);
    assert_eq!(
        supplied.to_string(),
        derived.to_string(),
        "feeding back `analyze_module`'s own graph must reproduce the derived expansion",
    );

    supplied.into()
}

/// Run the ranked engine with `contract: true`, stating the participants as *every `pub` type in the
/// module* — so nothing in the source has to spell the cycle.
///
/// This is the case `emit_contracts` exists for: a generating caller whose impls are written
/// with fully-qualified trait paths throughout. Without `contract` the engine adopts neither impl (a
/// qualified header is the documented opt-out) and the obligation cycle is left unbroken.
#[proc_macro_attribute]
#[proc_macro_error]
pub fn decycle_via_graph_contract(_attr: TokenStream, input: TokenStream) -> TokenStream {
    use decycle_impl::analysis::with_nodes;
    use decycle_impl::safegraph::VecGraph;

    let module = parse_macro_input!(input as ItemMod);
    let decycle: syn::Path = parse_quote!(::decycle);

    let participants: Vec<proc_macro2::Ident> = module
        .content
        .iter()
        .flat_map(|(_, items)| items)
        .filter_map(|item| match item {
            syn::Item::Enum(e) => Some(e.ident.clone()),
            syn::Item::Struct(st) => Some(st.ident.clone()),
            _ => None,
        })
        .collect();
    let graph = with_nodes(&VecGraph::default(), participants);

    process_module_with_graph(module, &graph, /* emit_contracts */ true, &decycle, 2, true).into()
}

/// Same as [`decycle_via_graph_contract`], but through the LOWER-level entry: build `FinalizeArgs`
/// by hand — the way a wrapper macro that has already split the module does — and call
/// `finalize_with_graph`. Proves the graph and `contract` reach the ranking machinery from there too.
#[proc_macro_attribute]
#[proc_macro_error]
pub fn finalize_via_graph_contract(_attr: TokenStream, input: TokenStream) -> TokenStream {
    use decycle_impl::analysis::with_nodes;
    use decycle_impl::finalize::{finalize_with_graph, FinalizeArgs};
    use decycle_impl::safegraph::VecGraph;

    let module = parse_macro_input!(input as ItemMod);
    let items: Vec<syn::Item> = module.content.into_iter().flat_map(|(_, i)| i).collect();

    let mut traits = Vec::new();
    let mut impls = Vec::new();
    let mut rest = Vec::new();
    let mut participants = Vec::new();
    for item in items {
        match item {
            syn::Item::Trait(mut t) => {
                t.attrs.retain(|a| !a.path().is_ident("decycle"));
                // The trait is emitted at module level AND described in `FinalizeArgs` — this is what
                // `process_module` does (it leaves the item in `raw_contents` while also collecting
                // it), and the ranked output refers to it from the enclosing scope.
                rest.push(syn::Item::Trait(t.clone()));
                traits.push(t);
            }
            syn::Item::Impl(im) if im.trait_.is_some() => impls.push(im),
            other => {
                if let syn::Item::Enum(e) = &other {
                    participants.push(e.ident.clone());
                } else if let syn::Item::Struct(st) = &other {
                    participants.push(st.ident.clone());
                }
                rest.push(other);
            }
        }
    }
    let graph = with_nodes(&VecGraph::default(), participants);

    let args = FinalizeArgs {
        working_list: vec![parse_quote!(::decycle::__finalize)],
        traits,
        contents: impls,
        recurse_level: 2,
        support_infinite_cycle: true,
        renames: Vec::new(),
        also_rank: Vec::new(),
        decycle_path: Some(parse_quote!(::decycle)),
    };
    let ident = &module.ident;
    let expanded = finalize_with_graph(args, &graph, /* emit_contracts */ true);
    let out = quote::quote! {
        mod #ident {
            #(#rest)*
            #expanded
        }
    };
    out.into()
}
