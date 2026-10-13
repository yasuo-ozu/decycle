use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::*;
use syn::parse::Parse;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;
use syn::*;
use template_quote::quote;
use type_leak::Leaker;

/// Apply `#[decycle]` to a trait definition, programmatically — for macro crates that define or
/// derive traits which should also be valid `#[decycle]` targets.
///
/// `marker_path` supplies the interning marker required when the trait contains non-absolute type
/// paths; `leaker_config` controls the allowed-path set for interning.
///
/// `mock = Some(path)` declares the trait a *stand-in* for the real trait at `path` (announce's
/// `mock`): the trait itself is not emitted, only its trampoline macro (named after the item, at
/// the item's visibility). The carried copy is announced with `mock = path`, so announce
/// type-checks it in a hidden module and lists `mock(path)` in the entry META; the `#[decycle]
/// use` site then treats it as the real trait (its ident becomes `path`'s last segment, so
/// `impl RealTrait for X` is matched). Spell `path` absolutely (`crate::..`, `::krate::..`).
///
/// The carrier is an [announce](https://docs.rs/announce) trampoline. The trait is emitted as written
/// (unless it is a stand-in); the interning and generic renaming run here, at the definition site,
/// and the rewritten copy is announced with `emit_item = false`, so announce evaluates its nested
/// cfgs and the trampoline carries the rewritten trait.
pub fn process_trait(
    trait_item: &ItemTrait,
    decycle_path: &Path,
    marker_path: Option<&Path>,
    mock: Option<&Path>,
    leaker_config: type_leak::LeakerConfig,
) -> TokenStream2 {
    let random_suffix = crate::get_random();
    let announce_path: Path = parse_quote!(#decycle_path::__private::announce);
    let crate_version = env!("CARGO_PKG_VERSION");
    // The rewritten copy is announced in announce's module-wrapper form: decycle makes its paths
    // resolve itself (interning into `Repeater` projections, `allowed_paths`, the marker), so
    // announce's own type-context check must not run on it. The wrapper is never emitted
    // (`emit_item = false`); `receive_carried_trait` unwraps it.
    //
    // A stand-in (`mock`) is not emitted at all — only the trampoline, under the item's name and
    // visibility. announce type-checks the mock copy inside a hidden module, where the wrapper is
    // one module deeper than the item was written: the leading `use super::*;` keeps the copy's
    // relative names (supertraits, the marker, `allowed_paths` roots) resolving there.
    // `receive_carried_trait` skips it.
    let wrapper_vis = &trait_item.vis;
    let wrapper_ident = &trait_item.ident;
    let carrier = |modified: &ItemTrait| {
        quote! {
            #(if mock.is_none()) { #trait_item }

            #[#announce_path::announce(
                crate = #announce_path,
                emit_item = false,
                #(if let Some(mock) = mock) { mock = #mock, }
                meta = __trait_meta(#crate_version)
            )]
            #(if mock.is_some()) { #[allow(non_snake_case)] }
            #wrapper_vis mod #wrapper_ident {
                #(if mock.is_some()) {
                    #[allow(unused_imports)]
                    use super::*;
                }
                #modified
            }
        }
    };
    let mut modified_trait_item = trait_item.clone();
    // Randomize Ident of GenericParam in modified_trait_item.generics
    let mut renamer =
        crate::randomize_impl_generics(&mut modified_trait_item.generics, random_suffix);
    renamer.visit_item_trait_mut(&mut modified_trait_item);
    // The carrier is built from `modified_trait_item`, which the interning rewrite below still has
    // to mutate — so it is a closure, called twice. Building it once up front (the old shape) meant
    // the final output re-used a stream captured BEFORE the rewrite: every interned type stayed
    // spelled as the caller wrote it, and the definition that travelled through the macro named
    // types that do not exist at the use site (`error[E0412]: cannot find type MyTy in this
    // scope`). The early call is only for `set_dummy`, so an abort in between still has a valid
    // fallback expansion.
    proc_macro_error::set_dummy(carrier(&modified_trait_item));

    let mut leaker = Leaker::from_config(leaker_config);
    leaker
        .intern_with(&trait_item.generics, |v| {
            v.visit_item_trait(trait_item);
        })
        .unwrap_or_else(|type_leak::NotInternableError(span)| {
            abort!(
                span,
                "decycle: this path cannot be carried through the #[decycle] trait's generated macro";
                help = "a #[decycle] trait definition is re-quoted into whatever module or crate routes it, where a relative path resolves differently (or not at all). Spell it absolutely (`::core::…`, `crate::…`), or add its root to `#[decycle(allowed_paths = [..])]` if it is always in scope at the use site."
            )
        });
    // `intern_with` only records pending graph operations; `reduce_roots` is what turns them into
    // the reachable-type set that `finish` reads. Without this call `finish` always returned an
    // EMPTY referrer, so nothing was ever interned, no `Repeater` impls were emitted, and the
    // `marker` argument was accepted and silently ignored (its "specify 'marker' arg" abort below
    // was unreachable) — the whole interning path was dead.
    leaker.reduce_roots();
    let referrer = leaker.finish();

    let typeref_impls = if !referrer.is_empty() {
        let marker_path = marker_path.unwrap_or_else(|| {
            abort!(
                Span::call_site(), "specify 'marker' arg";
                hint = referrer.iter().next().unwrap().span() => "first type to be interned"
            )
        });
        let encoded_ty =
            type_leak::encode_generics_params_to_ty(&modified_trait_item.generics.params);
        referrer
            .clone()
            .into_visitor(
                |_, num| parse_quote!(<#marker_path as #decycle_path::Repeater<#random_suffix, #num, #encoded_ty>>::Type),
            )
            .visit_item_trait_mut(&mut modified_trait_item);
        let impl_generics = modified_trait_item.generics.split_for_impl().0;
        referrer
            .iter()
            .enumerate()
            .map(|(ix, ty)| {
                quote! {
                    impl #impl_generics
                    #decycle_path::Repeater<#random_suffix, #ix, #encoded_ty> for #marker_path {
                        type Type = #ty;
                    }
                }
            })
            .collect()
    } else {
        quote!()
    };

    quote! {
        #{carrier(&modified_trait_item)}

        #typeref_impls
    }
}

/// Receive one trampoline entry at `__finalize`. A `#[decycle]` trait arrives already rewritten by
/// [`process_trait`], wrapped in a same-named module; its `__trait_meta("VERSION")` must match this
/// decycle. An entry without it
/// (a plain `#[announce]` trait) only gets its generic parameters renamed.
///
/// An entry with a `mock(path)` META (a `#[decycle(mock = path)]` stand-in, or a plain
/// `#[announce(mock = path)]` copy such as announce-std's) is the real trait at `path`: its ident
/// becomes `path`'s last segment, so the module's `impl RealTrait for X` is matched, and an
/// absolute `path` is recorded (see [`MOCK_ATTR`]) for the places that name the trait itself.
pub(crate) fn receive_carried_trait(entry: &announce::helper::Entry) -> ItemTrait {
    let not_a_trait = || -> ! {
        abort!(
            entry.item.span(),
            "#[decycle] use: `{}` is not a trait",
            template_quote::ToTokens::to_token_stream(&entry.key).to_string()
        )
    };
    // `#[decycle]` announces `mod T { <rewritten trait> }` (announce's module wrapper); a plain
    // `#[announce]` trait arrives as itself.
    let mut trait_item: ItemTrait = match syn::parse2::<Item>(entry.item.clone()) {
        Ok(Item::Trait(t)) => t,
        Ok(Item::Mod(ItemMod {
            content: Some((_, items)),
            ..
        })) => {
            // Exactly one trait; a stand-in's wrapper also holds a leading `use super::*;`.
            let mut items = items.into_iter().filter(|it| !matches!(it, Item::Use(_)));
            match (items.next(), items.next()) {
                (Some(Item::Trait(t)), None) => t,
                _ => not_a_trait(),
            }
        }
        _ => not_a_trait(),
    };
    match entry.find_meta("__trait_meta") {
        Some(meta) => {
            let version = meta
                .parse_args_with(<LitStr as Parse>::parse)
                .unwrap_or_else(|e| abort!(e.span(), "decycle: malformed trait metadata: {}", e));
            let expected_version = env!("CARGO_PKG_VERSION");
            if version.value() != expected_version {
                abort!(
                    Span::call_site(),
                    "version mismatch: expected '{}', got '{}'",
                    expected_version,
                    version.value()
                )
            }
        }
        None => {
            let mut renamer =
                crate::randomize_impl_generics(&mut trait_item.generics, crate::get_random());
            renamer.visit_item_trait_mut(&mut trait_item);
        }
    }
    if let Some(path) = entry.mock() {
        let real = path
            .segments
            .last()
            .unwrap_or_else(|| abort!(path, "decycle: empty `mock` path"));
        trait_item.ident = real.ident.clone();
        if is_absolute(&path) {
            let attr = Ident::new(MOCK_ATTR, Span::call_site());
            trait_item.attrs.push(parse_quote!(#[#attr(#path)]));
        }
    }
    trait_item
}

/// The name of the inert attribute [`receive_carried_trait`] puts on a carried stand-in trait to
/// record its real trait's absolute path. Carried traits' own attributes are never re-emitted by
/// `finalize`, so it only travels inside the `ItemTrait`; read it with [`mock_path`].
pub(crate) const MOCK_ATTR: &str = "__decycle_mock";

/// The real trait's absolute path of a carried stand-in trait (`mock(path)` META), if any.
pub(crate) fn mock_path(trait_item: &ItemTrait) -> Option<Path> {
    trait_item
        .attrs
        .iter()
        .find(|a| a.path().is_ident(MOCK_ATTR))
        .and_then(|a| a.parse_args::<Path>().ok())
}

/// `::a::b`, `crate::..` or `$crate::..` (what announce turns the definer's `crate::` into): a
/// path that names the same item from any module.
fn is_absolute(path: &Path) -> bool {
    path.leading_colon.is_some()
        || path
            .segments
            .first()
            .is_some_and(|s| s.ident == "crate" || s.ident == "$crate")
}
