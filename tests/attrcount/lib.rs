//! Test-only attribute macro that **counts its own expansions**.
//!
//! Neither engine may replicate an arbitrary attribute macro onto more than one of the copies it
//! makes of a method (`decycle_impl::propagated_method_attrs` says so in as many words: replicating
//! an attribute macro "would double-run [it] or duplicate items"). "Compiles fine" does not test
//! that — a double expansion of an idempotent attribute compiles fine — so this macro reports how
//! many times it actually ran.
//!
//! Each expansion takes the next ordinal **for its label** (`#[count_expansions(structural)]` and
//! `#[count_expansions(ranked)]` count separately) and bakes that compile-time ordinal into a call
//! prepended to the method body:
//!
//! ```ignore
//! crate::__decycle_attr_expansion_hit("structural", 1usize);
//! ```
//!
//! The test crate defines `__decycle_attr_expansion_hit` at its root and records the calls, so
//! calling the method once shows both how many instrumented copies exist on the call path and,
//! through the ordinal, how many times the attribute was expanded at compile time. Two hits — or one
//! hit whose ordinal is 2 — means the attribute ran twice.

use proc_macro::TokenStream;
use quote::quote;
use std::collections::BTreeMap;
use std::sync::Mutex;
use syn::{parse_macro_input, ImplItemFn, LitStr};

/// label → expansions so far. Proc-macro state lives as long as the compilation of one crate, which
/// is exactly the scope the counting is about.
static COUNTS: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());

/// Instrument a trait-impl method: `#[attrcount::count_expansions(<label>)]`.
#[proc_macro_attribute]
pub fn count_expansions(attr: TokenStream, item: TokenStream) -> TokenStream {
    let label = attr.to_string().trim().to_string();
    let ordinal = {
        let mut counts = COUNTS.lock().unwrap();
        let n = counts.entry(label.clone()).or_insert(0);
        *n += 1;
        *n
    };
    let mut f = parse_macro_input!(item as ImplItemFn);
    let label = LitStr::new(&label, proc_macro2::Span::call_site());
    let hit: syn::Stmt = syn::parse_quote! {
        crate::__decycle_attr_expansion_hit(#label, #ordinal);
    };
    f.block.stmts.insert(0, hit);
    quote!(#f).into()
}
