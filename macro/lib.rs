//! see document for [`decycle`](https://docs.rs/decycle) crate.

use proc_macro::{Span, TokenStream};
use proc_macro_error::*;
use syn::parse::{Parse, ParseStream};
use syn::*;
use template_quote::quote;

mod finalize;
mod process_module;
mod process_trait;
use process_module::process_module;
use process_trait::process_trait;

struct Args {
    decycle: Option<Path>,
    marker: Option<Path>,
    recurse_level: Option<usize>,
    support_infinite_cycle: Option<bool>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut args = Args {
            decycle: None,
            marker: None,
            recurse_level: None,
            support_infinite_cycle: None,
        };
        syn::custom_keyword!(decycle);
        syn::custom_keyword!(marker);
        syn::custom_keyword!(recurse_level);
        syn::custom_keyword!(support_infinite_cycle);
        while !input.is_empty() {
            let lookahead = input.lookahead1();
            if lookahead.peek(decycle) {
                input.parse::<decycle>()?;
                input.parse::<Token![=]>()?;
                args.decycle = Some(input.parse()?);
            } else if lookahead.peek(marker) {
                input.parse::<marker>()?;
                input.parse::<Token![=]>()?;
                args.marker = Some(input.parse()?);
            } else if lookahead.peek(recurse_level) {
                input.parse::<recurse_level>()?;
                input.parse::<Token![=]>()?;
                let lit: LitInt = input.parse()?;
                args.recurse_level = Some(lit.base10_parse()?);
            } else if lookahead.peek(support_infinite_cycle) {
                input.parse::<support_infinite_cycle>()?;
                input.parse::<Token![=]>()?;
                let lit: LitBool = input.parse()?;
                args.support_infinite_cycle = Some(lit.value);
            } else {
                abort!(
                    input.span(),
                    "keyword arguments should be one of 'decycle', 'marker', 'recurse_level', 'support_infinite_cycle'"
                )
            }
            if input.parse::<Token![,]>().is_err() {
                break;
            }
        }
        Ok(args)
    }
}

#[proc_macro_error]
#[proc_macro_attribute]
pub fn decycle(attr: TokenStream, input: TokenStream) -> TokenStream {
    proc_macro_error::set_dummy(input.clone().into());
    let args = parse_macro_input!(attr as Args);
    let decycle_path = args.decycle.unwrap_or_else(|| parse_quote!(::decycle));

    if let Ok(module) = parse::<ItemMod>(input.clone()) {
        let recurse_level = args.recurse_level.unwrap_or(10);
        let support_infinite_cycle = args.support_infinite_cycle.unwrap_or(true);
        let ret = std::panic::catch_unwind(|| {
            process_module(module, &decycle_path, recurse_level, support_infinite_cycle)
        })
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
        proc_macro_error::set_dummy(quote!(#ret));
        if let Some(marker) = &args.marker {
            abort!(marker, "unsupported argument 'marker'")
        }
        ret.into()
    } else if let Ok(item) = parse::<ItemTrait>(input.clone()) {
        let ret =
            std::panic::catch_unwind(|| process_trait(&item, &decycle_path, args.marker.as_ref()))
                .unwrap_or_else(|e| std::panic::resume_unwind(e));
        proc_macro_error::set_dummy(quote!(#ret));
        if let Some(_) = args.recurse_level {
            abort!(
                Span::call_site(),
                "recurse_level is not supported for trait items"
            )
        }
        if let Some(_) = args.support_infinite_cycle {
            abort!(
                Span::call_site(),
                "support_infinite_cycle is not supported for trait items"
            )
        }
        ret.into()
    } else if let Ok(mut item_use) = parse::<ItemUse>(input.clone()) {
        item_use.attrs.clear();
        abort!(
            Span::call_site(),
            "place it inside module annotated with #[decycle]";
            hint = r#"
            Example:
            #[decycle]
            mod some_module {{
                #[decycle]
                {}
            }}
         "#, quote!(#item_use).to_string()
        )
    } else {
        abort!(
            Span::call_site(),
            "not supported";
            hint = "#[decycle] supports module or trait"
        )
    }
}

#[doc(hidden)]
#[proc_macro]
pub fn __finalize(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input as finalize::FinalizeArgs);
    finalize::finalize(args).into()
}

fn is_decycle_attribute(attr: &Attribute) -> bool {
    let path = &attr.path();
    path.is_ident("decycle")
        || (path.segments.len() == 2
            && (&path.segments[0].ident == "decycle"
                || is_renamed_decycle_crate(&path.segments[0].ident))
            && &path.segments[1].ident == "decycle")
}

fn is_renamed_decycle_crate(ident: &proc_macro2::Ident) -> bool {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();

    if let Ok(cargo_toml) = std::fs::read_to_string(format!("{}/Cargo.toml", manifest_dir)) {
        if cargo_toml.contains(&format!("{} = {{ package = \"decycle\"", ident))
            || cargo_toml.contains(&format!("{} = {{ package = 'decycle'", ident))
        {
            return true;
        }
    }

    false
}

fn ident_to_path(ident: &Ident) -> Path {
    Path {
        leading_colon: None,
        segments: core::iter::once(PathSegment {
            ident: ident.clone(),
            arguments: PathArguments::None,
        })
        .collect(),
    }
}

fn get_random() -> u64 {
    use core::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}
