//! see document for [`decycle`](https://docs.rs/decycle) crate.

use decycle_impl::proc_macro_error::*;
use proc_macro::{Span, TokenStream};
use syn::parse::{Parse, ParseStream};
use syn::*;
use template_quote::quote;

use decycle_impl::process_module;
use decycle_impl::process_trait;

struct Args {
    decycle: Option<Path>,
    marker: Option<Path>,
    alter_macro_name: Option<Ident>,
    allowed_paths: Option<Vec<Path>>,
    recurse_level: Option<usize>,
    support_infinite_cycle: Option<bool>,
}

impl Parse for Args {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut args = Args {
            decycle: None,
            marker: None,
            alter_macro_name: None,
            allowed_paths: None,
            recurse_level: None,
            support_infinite_cycle: None,
        };
        syn::custom_keyword!(decycle);
        syn::custom_keyword!(marker);
        syn::custom_keyword!(alter_macro_name);
        syn::custom_keyword!(allowed_paths);
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
            } else if lookahead.peek(alter_macro_name) {
                input.parse::<alter_macro_name>()?;
                input.parse::<Token![=]>()?;
                args.alter_macro_name = Some(input.parse()?);
            } else if lookahead.peek(allowed_paths) {
                input.parse::<allowed_paths>()?;
                input.parse::<Token![=]>()?;
                let content;
                bracketed!(content in input);
                let paths = content.parse_terminated(Path::parse, Token![,])?;
                args.allowed_paths = Some(paths.into_iter().collect());
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
                    "keyword arguments should be one of 'decycle', 'marker', 'alter_macro_name', 'allowed_paths', 'recurse_level', 'support_infinite_cycle'"
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
    set_dummy(input.clone().into());
    let args = parse_macro_input!(attr as Args);
    let decycle_path = args.decycle.unwrap_or_else(|| parse_quote!(::decycle));

    if let Ok(module) = parse::<ItemMod>(input.clone()) {
        let recurse_level = args.recurse_level.unwrap_or(10);
        let support_infinite_cycle = args.support_infinite_cycle.unwrap_or(true);
        let ret = std::panic::catch_unwind(|| {
            process_module(module, &decycle_path, recurse_level, support_infinite_cycle)
        })
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
        set_dummy(quote!(#ret));
        if let Some(marker) = &args.marker {
            abort!(marker, "unsupported argument 'marker'")
        }
        if let Some(alter_macro_name) = &args.alter_macro_name {
            abort!(alter_macro_name, "unsupported argument 'alter_macro_name'")
        }
        ret.into()
    } else if let Ok(item) = parse::<ItemTrait>(input.clone()) {
        let mut config = type_leak::LeakerConfig::new();
        if let Some(paths) = &args.allowed_paths {
            config.allowed_paths.extend(paths.clone());
        } else {
            config.allow_crate();
            config.allow_primitive();
        }
        let ret = std::panic::catch_unwind(|| {
            process_trait(
                &item,
                &decycle_path,
                args.marker.as_ref(),
                args.alter_macro_name.as_ref(),
                config,
            )
        })
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
        set_dummy(quote!(#ret));
        if args.recurse_level.is_some() {
            abort!(
                Span::call_site(),
                "recurse_level is not supported for trait items"
            )
        }
        if args.support_infinite_cycle.is_some() {
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
    let args = parse_macro_input!(input as decycle_impl::finalize::FinalizeArgs);
    decycle_impl::finalize::finalize(args).into()
}
