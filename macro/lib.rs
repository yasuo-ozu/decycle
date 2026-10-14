//! see document for [`decycle`](https://docs.rs/decycle) crate.

use decycle_impl::proc_macro_error::*;
use proc_macro::{Span, TokenStream};
use syn::parse::{Parse, ParseStream};
use syn::*;
use template_quote::quote;

use decycle_impl::process_module;
use decycle_impl::process_module_structural;
use decycle_impl::{check_trait, check_trait_structural};

struct Args {
    decycle: Option<Path>,
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
            recurse_level: None,
            support_infinite_cycle: None,
            structural: false,
            duplicate: None,
        };
        syn::custom_keyword!(decycle);
        syn::custom_keyword!(marker);
        syn::custom_keyword!(mock);
        syn::custom_keyword!(alter_macro_name);
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
            } else if input.peek(marker) {
                let kw: marker = input.parse()?;
                abort!(
                    kw.span,
                    "`marker` was an option of `#[decycle]` on a trait, which was removed in 0.6";
                    help = "nothing replaces it: `#[announce]` requires every path in the trait to resolve outside the defining module, so spell them absolutely (`crate::..`, `::krate::..`)"
                )
            } else if input.peek(mock) {
                let kw: mock = input.parse()?;
                abort!(
                    kw.span,
                    "`mock` was an option of `#[decycle]` on a trait, which was removed in 0.6";
                    help = "announce the trait with `#[::decycle::announce::announce(crate = ::decycle::announce, mock = path::to::RealTrait)]`"
                )
            } else if input.peek(alter_macro_name) {
                let kw: alter_macro_name = input.parse()?;
                abort!(
                    kw.span,
                    "`alter_macro_name` was an option of `#[decycle]` on a trait, which was removed in 0.6";
                    help = "nothing replaces it: `#[decycle] use` the trait by its own name"
                )
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
                    "keyword arguments should be one of 'decycle', 'recurse_level', 'support_infinite_cycle', 'structural'"
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

/// Is `attr` a `#[decycle]` marker, in either spelling: bare, or two-segment `<x>::decycle`? The
/// args are not known where this is used, so `<x>` is not tied to one crate name.
fn names_decycle(attr: &Attribute) -> bool {
    let path = attr.path();
    path.is_ident("decycle") || (path.segments.len() == 2 && path.segments[1].ident == "decycle")
}

/// Drop the `#[decycle]` markers from the direct items of `module`, for the dummy set before the
/// arguments are parsed.
///
/// Without it, an abort while parsing the arguments leaves the markers in the dummy, where each
/// one adds a "cannot find attribute `decycle`" error to the one the user needs to see.
fn strip_inner_decycle_attrs(module: &mut ItemMod) {
    if let Some((_, items)) = module.content.as_mut() {
        for item in items {
            let attrs = match item {
                Item::Trait(ItemTrait { attrs, .. })
                | Item::TraitAlias(ItemTraitAlias { attrs, .. })
                | Item::Use(ItemUse { attrs, .. })
                | Item::Mod(ItemMod { attrs, .. }) => attrs,
                _ => continue,
            };
            attrs.retain(|attr| !names_decycle(attr));
        }
    }
}

#[proc_macro_error]
#[proc_macro_attribute]
pub fn decycle(attr: TokenStream, input: TokenStream) -> TokenStream {
    let module = parse::<ItemMod>(input.clone()).ok();
    set_dummy(match &module {
        Some(module) if !module.attrs.iter().any(names_decycle) => {
            let mut module = module.clone();
            strip_inner_decycle_attrs(&mut module);
            quote!(#module)
        }
        // Applied twice: the remaining outer `#[decycle]` expands the module with its markers.
        _ => input.clone().into(),
    });
    let args = parse_macro_input!(attr as Args);
    let decycle_path = args.decycle.clone().unwrap_or_else(|| parse_quote!(::decycle));

    if let Some(module) = module {
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
        ret.into()
    } else if let Ok(item_trait) = parse::<ItemTrait>(input.clone()) {
        // The dummy is the raw trait here, so each abort below stays a single error.
        if let Some(span) = pending_decycle_attr(&item_trait.attrs, &decycle_path) {
            abort!(span, "#[decycle] is already applied to this trait")
        }
        // The engine options belong to the module; on a trait they would be dropped silently.
        let module_only = if args.structural {
            Some("structural")
        } else if args.recurse_level.is_some() {
            Some("recurse_level")
        } else if args.support_infinite_cycle.is_some() {
            Some("support_infinite_cycle")
        } else {
            None
        };
        if let Some(name) = module_only {
            abort!(
                Span::call_site(),
                "`{}` applies to a `#[decycle]` module, not a trait", name;
                help = "write it on the enclosing `#[decycle] mod`"
            )
        }
        reject_duplicate(&args);
        // A trait outside a `#[decycle] mod` is announced, so that a `#[decycle] use` (in this crate
        // or another) can carry it into a module. The std prelude names (`Option`, `String`,
        // `Clone`, ..) stay accepted, as they were before the trait was announced.
        let item = proc_macro2::TokenStream::from(input);
        quote! {
            #[#decycle_path::announce::announce(crate = #decycle_path::announce, allow_std_prelude = true)]
            #item
        }
        .into()
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
            hint = "#[decycle] supports an inline module"
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

/// The implementation of `decycle::check`: validates a trait at its definition (see the docs of
/// `decycle::check`). Run by `#[announce(.., decycle::check)]` as `check! { (), TRAIT }`, and by
/// `#[announce(.., decycle::check(structural))]` as `check! { (structural), TRAIT }`.
#[proc_macro_error]
#[proc_macro]
pub fn check(input: TokenStream) -> TokenStream {
    announce::helper::check(input.into(), &[], |c| {
        let structural = parse_check_args(&c.args)?;
        match &c.item {
            Item::Trait(item_trait) => {
                if structural {
                    check_trait_structural(item_trait);
                } else {
                    check_trait(item_trait);
                }
                Ok(proc_macro2::TokenStream::new())
            }
            other => Err(Error::new(
                // The first token of the item (a single token, so that the position does not depend
                // on whether the compiler can join spans).
                template_quote::ToTokens::to_token_stream(other)
                    .into_iter()
                    .next()
                    .map_or_else(proc_macro2::Span::call_site, |t| t.span()),
                "`decycle::check` applies to a trait definition",
            )),
        }
    })
    .into()
}

/// The arguments of `decycle::check(..)`: nothing (the rules of the default ranked engine), or
/// `structural` (the rules of `#[decycle(structural)]`). Returns whether it was `structural`.
fn parse_check_args(args: &proc_macro2::TokenStream) -> Result<bool> {
    if args.is_empty() {
        return Ok(false);
    }
    syn::custom_keyword!(structural);
    syn::parse2::<structural>(args.clone()).map(|_| true).map_err(|e| {
        Error::new(
            e.span(),
            "`decycle::check` takes no argument, or `structural` (`decycle::check(structural)`)",
        )
    })
}
