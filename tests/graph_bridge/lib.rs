//! Test-only bridge for the graph-taking engine entry points.
//!
//! `ranked::process_module_with_graph` cannot be exercised from a plain integration test: the ranked
//! engine reports through `proc_macro_error`, whose API panics outside a proc-macro entry point. So
//! the call is made here, inside a real `#[proc_macro_error]` function, exactly the way a wrapper
//! macro crate would make it.

use decycle_impl::analysis::analyze_module;
use proc_macro_error::proc_macro_error;
use decycle_impl::ranked::{process_module, process_module_with_graph, GraphOptions};
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
        GraphOptions::default(),
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
/// This is the case `GraphOptions::contract` exists for: a generating caller whose impls are written
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

    process_module_with_graph(
        module,
        &graph,
        GraphOptions { contract: true },
        &decycle,
        2,
        true,
    )
    .into()
}
