//! Test-only bridge for the ranked engine's programmatic entry point.
//!
//! `ranked::process_module` cannot be called from a plain integration test: the ranked engine
//! reports through `proc_macro_error`, whose API panics outside a proc-macro entry point. So the
//! call is made here, inside a real `#[proc_macro_error]` function, exactly the way a wrapper macro
//! crate would make it.

use decycle_impl::ranked::process_module;
use proc_macro::TokenStream;
use proc_macro_error::proc_macro_error;
use syn::{parse_macro_input, parse_quote, ItemMod};

/// Expand the module with the ranked engine several times over, in this one process, and assert
/// that every expansion is the same token stream; the output is the first one.
///
/// Each `HashMap` a run builds hashes with keys of its own, so iterating one where tokens are
/// emitted shows up here as two runs that differ (and, across compilations, as a module whose
/// expansion changes from one build to the next).
#[proc_macro_attribute]
#[proc_macro_error]
pub fn expand_repeatedly(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let module = parse_macro_input!(input as ItemMod);
    let decycle: syn::Path = parse_quote!(::decycle);

    let first = process_module(module.clone(), &decycle, 2, true);
    for run in 1..8 {
        let again = process_module(module.clone(), &decycle, 2, true);
        assert_eq!(
            first.to_string(),
            again.to_string(),
            "expansion {run} of the same module differs from the first"
        );
    }
    first.into()
}
