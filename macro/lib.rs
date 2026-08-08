//! see document for [`decycle`](https://docs.rs/decycle) crate.

use decycle_impl::proc_macro_error::*;
use proc_macro::{Span, TokenStream};
use syn::parse::{Parse, ParseStream};
use syn::*;
use template_quote::quote;

use decycle_impl::process_module;
use decycle_impl::process_module_structural;
use decycle_impl::process_trait;

struct Args {
    decycle: Option<Path>,
    marker: Option<Path>,
    alter_macro_name: Option<Ident>,
    allowed_paths: Option<Vec<Path>>,
    recurse_level: Option<usize>,
    support_infinite_cycle: Option<bool>,
    structural: bool,
    /// First repeated keyword seen: the SECOND occurrence's span, plus the keyword name. A repeat
    /// used to silently last-win; it is now rejected — but only AFTER the item is processed and
    /// `set_dummy` holds a valid expansion, so the abort does not cascade (see `reject_duplicate`).
    duplicate: Option<(proc_macro2::Span, &'static str)>,
}

impl Args {
    /// Record a repeated keyword (keeping the FIRST value, purely so parsing can finish — the
    /// duplicate is rejected before any of it matters).
    fn note_duplicate(&mut self, span: proc_macro2::Span, name: &'static str) {
        if self.duplicate.is_none() {
            self.duplicate = Some((span, name));
        }
    }
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
            structural: false,
            duplicate: None,
        };
        syn::custom_keyword!(decycle);
        syn::custom_keyword!(marker);
        syn::custom_keyword!(alter_macro_name);
        syn::custom_keyword!(allowed_paths);
        syn::custom_keyword!(recurse_level);
        syn::custom_keyword!(support_infinite_cycle);
        syn::custom_keyword!(structural);
        while !input.is_empty() {
            let lookahead = input.lookahead1();
            if lookahead.peek(structural) {
                let kw: structural = input.parse()?;
                if args.structural {
                    args.note_duplicate(kw.span, "structural");
                }
                args.structural = true;
            } else if lookahead.peek(decycle) {
                let kw: decycle = input.parse()?;
                input.parse::<Token![=]>()?;
                let value = input.parse()?;
                if args.decycle.is_some() {
                    args.note_duplicate(kw.span, "decycle");
                } else {
                    args.decycle = Some(value);
                }
            } else if lookahead.peek(marker) {
                let kw: marker = input.parse()?;
                input.parse::<Token![=]>()?;
                let value = input.parse()?;
                if args.marker.is_some() {
                    args.note_duplicate(kw.span, "marker");
                } else {
                    args.marker = Some(value);
                }
            } else if lookahead.peek(alter_macro_name) {
                let kw: alter_macro_name = input.parse()?;
                input.parse::<Token![=]>()?;
                let value = input.parse()?;
                if args.alter_macro_name.is_some() {
                    args.note_duplicate(kw.span, "alter_macro_name");
                } else {
                    args.alter_macro_name = Some(value);
                }
            } else if lookahead.peek(allowed_paths) {
                let kw: allowed_paths = input.parse()?;
                input.parse::<Token![=]>()?;
                let content;
                bracketed!(content in input);
                let paths = content.parse_terminated(Path::parse, Token![,])?;
                if args.allowed_paths.is_some() {
                    args.note_duplicate(kw.span, "allowed_paths");
                } else {
                    args.allowed_paths = Some(paths.into_iter().collect());
                }
            } else if lookahead.peek(recurse_level) {
                let kw: recurse_level = input.parse()?;
                input.parse::<Token![=]>()?;
                let lit: LitInt = input.parse()?;
                if args.recurse_level.is_some() {
                    args.note_duplicate(kw.span, "recurse_level");
                } else {
                    args.recurse_level = Some(lit.base10_parse()?);
                }
            } else if lookahead.peek(support_infinite_cycle) {
                let kw: support_infinite_cycle = input.parse()?;
                input.parse::<Token![=]>()?;
                let lit: LitBool = input.parse()?;
                if args.support_infinite_cycle.is_some() {
                    args.note_duplicate(kw.span, "support_infinite_cycle");
                } else {
                    args.support_infinite_cycle = Some(lit.value);
                }
            } else {
                abort!(
                    input.span(),
                    "keyword arguments should be one of 'decycle', 'marker', 'alter_macro_name', 'allowed_paths', 'recurse_level', 'support_infinite_cycle', 'structural'"
                )
            }
            if input.parse::<Token![,]>().is_err() {
                break;
            }
        }
        Ok(args)
    }
}

/// Reject the first repeated keyword argument, pointing at its SECOND occurrence.
///
/// Called only after `set_dummy` holds the processed item, so the abort emits exactly one error —
/// the same no-cascade ordering the unsupported-argument rejections already follow.
fn reject_duplicate(args: &Args) {
    if let Some((span, name)) = &args.duplicate {
        abort!(span, "duplicate argument '{}'", name);
    }
}

/// The span of a still-pending `#[decycle]` attribute on the item itself — the shape left behind
/// when `#[decycle]` is written TWICE on one item: attribute macros expand outermost-first, so the
/// outer invocation sees the inner attribute verbatim in the item's `attrs`. Letting it through
/// used to fail incomprehensibly later (the inner expansion trips over the outer's generated
/// paths on a module, and redefines names on a trait — E0252 with an unusable rustc suggestion).
///
/// Matches the same spellings `decycle_impl::is_decycle_attribute` accepts on inner items: bare
/// `#[decycle]`, or two-segment `#[<crate>::decycle]` where `<crate>` is `decycle` or the leading
/// segment of the `decycle = …` path argument.
fn pending_decycle_attr(attrs: &[Attribute], decycle_path: &Path) -> Option<proc_macro2::Span> {
    let decycle_crate = decycle_path.segments.first().map(|seg| &seg.ident);
    attrs.iter().find_map(|attr| {
        let path = attr.path();
        let matched = path.is_ident("decycle")
            || (path.segments.len() == 2
                && (path.segments[0].ident == "decycle"
                    || Some(&path.segments[0].ident) == decycle_crate)
                && path.segments[1].ident == "decycle");
        matched.then(|| syn::spanned::Spanned::span(attr))
    })
}

#[proc_macro_error]
#[proc_macro_attribute]
pub fn decycle(attr: TokenStream, input: TokenStream) -> TokenStream {
    set_dummy(input.clone().into());
    let args = parse_macro_input!(attr as Args);
    let decycle_path = args.decycle.clone().unwrap_or_else(|| parse_quote!(::decycle));

    if let Ok(module) = parse::<ItemMod>(input.clone()) {
        // Applied twice? Abort here, where it can still be said plainly. The dummy is the raw
        // input, whose remaining `#[decycle]` will expand the original module cleanly — so this
        // stays a single error.
        if let Some(span) = pending_decycle_attr(&module.attrs, &decycle_path) {
            abort!(span, "#[decycle] is already applied to this module")
        }
        // Fail closed up front, for BOTH engines, on an empty `#[decycle]` module: a bodyless
        // `mod m;` or an empty `mod m {}` has nothing to expand.
        if module
            .content
            .as_ref()
            // `map_or(true, …)`, not `is_none_or`: the latter is stable only since 1.82 and
            // this crate's MSRV is 1.71.
            .map_or(true, |(_, items)| items.is_empty())
        {
            abort!(
                Span::call_site(),
                "#[decycle] requires an inline module with at least one item"
            )
        }
        let ret = if args.structural {
            // The structural unroll has no rank floor, so the depth/infinite knobs don't apply.
            if args.recurse_level.is_some() || args.support_infinite_cycle.is_some() {
                abort!(
                    Span::call_site(),
                    "recurse_level / support_infinite_cycle are not supported with `structural`"
                )
            }
            process_module_structural(module, &decycle_path)
        } else {
            let recurse_level = args.recurse_level.unwrap_or(10);
            if recurse_level == 0 {
                abort!(
                    Span::call_site(),
                    "recurse_level must be at least 1";
                    hint = "at level 0 the delegating impl would dispatch straight to the rank floor"
                )
            }
            let support_infinite_cycle = args.support_infinite_cycle.unwrap_or(true);
            process_module(module, &decycle_path, recurse_level, support_infinite_cycle)
        };
        // Process FIRST so the dummy fallback is the (valid) expanded module, THEN reject unsupported
        // args — matching the original ordering so an unsupported-arg abort doesn't cascade.
        set_dummy(quote!(#ret));
        reject_duplicate(&args);
        if let Some(marker) = &args.marker {
            abort!(marker, "unsupported argument 'marker'")
        }
        if let Some(alter_macro_name) = &args.alter_macro_name {
            abort!(alter_macro_name, "unsupported argument 'alter_macro_name'")
        }
        if args.allowed_paths.is_some() {
            abort!(
                Span::call_site(),
                "allowed_paths is not supported for modules"
            )
        }
        ret.into()
    } else if let Ok(item) = parse::<ItemTrait>(input.clone()) {
        // Applied twice? Same detection as the module branch, same reasoning: the outer invocation
        // sees the inner attribute still attached and would otherwise expand right over it,
        // re-defining the trait name (E0252) with a rustc suggestion that isn't valid Rust.
        if let Some(span) = pending_decycle_attr(&item.attrs, &decycle_path) {
            abort!(span, "#[decycle] is already applied to this trait")
        }
        let mut config = type_leak::LeakerConfig::new();
        if let Some(paths) = &args.allowed_paths {
            config.allowed_paths.extend(paths.clone());
        } else {
            config.allow_crate();
            config.allow_primitive();
        }
        let ret = process_trait(
            &item,
            &decycle_path,
            args.marker.as_ref(),
            args.alter_macro_name.as_ref(),
            config,
        );
        set_dummy(quote!(#ret));
        reject_duplicate(&args);
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
        if args.structural {
            abort!(
                Span::call_site(),
                "structural is not supported for trait items";
                hint = "structural selects the module-level engine; write it on the enclosing #[decycle(structural)] mod instead"
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
#[proc_macro_error]
pub fn __finalize(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input as decycle_impl::finalize::FinalizeArgs);
    decycle_impl::finalize::finalize(args).into()
}
