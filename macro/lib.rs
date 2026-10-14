//! see document for [`decycle`](https://docs.rs/decycle) crate.

use decycle_impl::proc_macro_error::*;
use proc_macro::{Span, TokenStream};
use syn::parse::discouraged::Speculative;
use syn::parse::{Parse, ParseStream};
use syn::*;
use template_quote::quote;

use decycle_impl::discover;
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
    /// First option of the removed trait-side `#[decycle]` (see `abort_removed_option`): its span and
    /// name. Like a repeat, it is rejected only after the item is processed, so the abort does not
    /// cascade.
    removed: Option<(proc_macro2::Span, &'static str)>,
}

impl Args {
    /// Record a repeated keyword (keeping the FIRST value, purely so parsing can finish — the
    /// duplicate is rejected before any of it matters).
    fn note_duplicate(&mut self, span: proc_macro2::Span, name: &'static str) {
        if self.duplicate.is_none() {
            self.duplicate = Some((span, name));
        }
    }

    /// Record an option of the removed trait-side `#[decycle]`, skipping its `= value`.
    ///
    /// The value is read as a type (then an expression) before anything else, because a comma inside
    /// it does not end the option: `marker = M<u8, u16>`. Stopping at the first top-level comma
    /// would leave `u16>` behind and replace the "removed in 0.6" help with a generic parse error.
    /// If it is neither, the rest of the arguments is dropped: the module or trait is rejected for
    /// this option anyway.
    fn note_removed(
        &mut self,
        input: ParseStream,
        span: proc_macro2::Span,
        name: &'static str,
    ) -> Result<()> {
        if self.removed.is_none() {
            self.removed = Some((span, name));
        }
        if input.parse::<Option<Token![=]>>()?.is_some() {
            let fork = input.fork();
            if fork.parse::<Type>().is_ok() {
                input.advance_to(&fork);
            } else {
                let fork = input.fork();
                if fork.parse::<Expr>().is_ok() {
                    input.advance_to(&fork);
                } else {
                    while !input.is_empty() {
                        input.parse::<proc_macro2::TokenTree>()?;
                    }
                }
            }
            // Anything left of the value, up to the next argument.
            while !input.is_empty() && !input.peek(Token![,]) {
                input.parse::<proc_macro2::TokenTree>()?;
            }
        }
        Ok(())
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
            removed: None,
        };
        syn::custom_keyword!(decycle);
        syn::custom_keyword!(marker);
        syn::custom_keyword!(mock);
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
            } else if input.peek(marker) {
                let kw: marker = input.parse()?;
                args.note_removed(input, kw.span, "marker")?;
            } else if input.peek(mock) {
                let kw: mock = input.parse()?;
                args.note_removed(input, kw.span, "mock")?;
            } else if input.peek(allowed_paths) {
                let kw: allowed_paths = input.parse()?;
                args.note_removed(input, kw.span, "allowed_paths")?;
            } else if input.peek(alter_macro_name) {
                let kw: alter_macro_name = input.parse()?;
                args.note_removed(input, kw.span, "alter_macro_name")?;
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

/// `#[decycle]` on a trait outside a `#[decycle] mod` was removed in 0.6: such a trait is announced
/// with `#[announce]`. The options of that form (`marker`, `mock`, `allowed_paths`, and 0.5's
/// `alter_macro_name`) go with it; this says so, and says what replaces each.
fn abort_removed_option(span: proc_macro2::Span, name: &str) -> ! {
    let help = match name {
        "mock" => "announce the trait with `#[::decycle::announce::announce(crate = ::decycle::announce, mock = path::to::RealTrait)]`",
        "alter_macro_name" => "announce the trait with `#[::decycle::announce::announce(crate = ::decycle::announce, macro_name = Name)]` to rename its trampoline (a `#[decycle] mod` then needs `use path::to::Trait;` and `#[decycle] use path::to::Name;`), or with `mock = path::to::RealTrait` to declare a stand-in for another trait",
        _ => "nothing replaces it: `#[announce]` requires every path in the trait to resolve outside the defining module, so spell them absolutely (`crate::..`, `::krate::..`)",
    };
    abort!(
        span,
        "`{}` was an option of `#[decycle]` on a trait, which was removed in 0.6", name;
        help = help
    )
}

/// Reject an option of the removed trait-side `#[decycle]`, if one was written.
fn reject_removed(args: &Args) {
    if let Some((span, name)) = args.removed {
        abort_removed_option(span, name)
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
            // A module whose `#[decycle] use` does not say which trait it carries learns it from the
            // trampoline first (`__discover`); any other is expanded at once.
            match discover::start(&module, &decycle_path, discover::Engine::Structural) {
                Some(ret) => ret,
                None => process_module_structural(module, &decycle_path),
            }
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
            let engine = discover::Engine::Ranked {
                recurse_level,
                support_infinite_cycle,
            };
            match discover::start(&module, &decycle_path, engine) {
                Some(ret) => ret,
                None => process_module(module, &decycle_path, recurse_level, support_infinite_cycle),
            }
        };
        // Process FIRST so the dummy fallback is the (valid) expanded module, THEN reject unsupported
        // args — matching the original ordering so an unsupported-arg abort doesn't cascade.
        set_dummy(quote!(#ret));
        reject_duplicate(&args);
        reject_removed(&args);
        ret.into()
    } else if parse::<ItemTrait>(input.clone()).is_ok() {
        // Removed in 0.6: a trait outside a `#[decycle] mod` is announced with `#[announce]`. The
        // dummy is the trait itself, so this stays a single error. An option of the old form is
        // named, with what replaces it.
        reject_removed(&args);
        abort!(
            Span::call_site(),
            "`#[decycle]` on a trait outside a `#[decycle] mod` was removed in 0.6";
            help = "announce the trait instead: `#[::decycle::announce::announce(crate = ::decycle::announce)]` (add `mock = path` for a stand-in, `::decycle::check` to validate it at its definition); `#[decycle] use` brings it into a `#[decycle] mod` as before"
        )
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

/// The callback of a module whose `#[decycle] use` needed the trampolines for the names of its
/// traits (`decycle_impl::discover`): expands the rest of the module. Called by `#[decycle]`.
#[doc(hidden)]
#[proc_macro]
#[proc_macro_error]
pub fn __discover(input: TokenStream) -> TokenStream {
    discover::finish(input.into()).into()
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
