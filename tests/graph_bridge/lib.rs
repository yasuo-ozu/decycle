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
    let supplied = process_module_with_graph(module.clone(), &graph, &decycle, 2, true);

    // Same input through the deriving entry point: the round trip has to be exact.
    let derived = process_module(module, &decycle, 2, true);
    assert_eq!(
        supplied.to_string(),
        derived.to_string(),
        "feeding back `analyze_module`'s own graph must reproduce the derived expansion",
    );

    supplied.into()
}
