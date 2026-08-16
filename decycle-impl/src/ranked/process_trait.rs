use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::*;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;
use syn::*;
use template_quote::quote;
use type_leak::Leaker;

/// Apply `#[decycle]` to a trait definition, programmatically — for macro crates that define or
/// derive traits which should also be valid `#[decycle]` targets.
///
/// `marker_path` supplies the interning marker required when the trait contains non-absolute type
/// paths; `alter_macro_name` renames the generated carrier macro; `leaker_config` controls the
/// allowed-path set for interning.
pub fn process_trait(
    trait_item: &ItemTrait,
    decycle_path: &Path,
    marker_path: Option<&Path>,
    alter_macro_name: Option<&Ident>,
    leaker_config: type_leak::LeakerConfig,
) -> TokenStream2 {
    let random_suffix = crate::get_random();
    let temporal_mac_name = alter_macro_name.cloned().unwrap_or_else(|| {
        // `random_suffix` is deterministic (hashed from the fixed crate identity string), and
        // `#[macro_export]` hoists this name to the crate root, so two `#[decycle] trait Foo`
        // items in one crate would otherwise compute the same name and collide (E0428). Fold a
        // per-invocation discriminant into THIS name only; `random_suffix` itself must stay
        // untouched everywhere else, since it's the key the leaker's `Repeater` impls (built
        // independently, elsewhere in this fn) are pinned to.
        //
        // The discriminant used to be a hash of the trait's own TOKENS, which cannot separate the
        // very case it exists for: two token-identical traits in different modules hash the same.
        // It is a fresh random value per invocation instead — see `fresh_invocation_id` for why
        // that is both safe here and preferable to a counter under incremental compilation.
        let discriminant = crate::fresh_invocation_id();
        syn::Ident::new(
            &format!(
                "__{}_temporal_{}_{}",
                &trait_item.ident, random_suffix, discriminant
            ),
            trait_item.ident.span(),
        )
    });
    let crate_version = env!("CARGO_PKG_VERSION");
    let crate_identity = LitStr::new(&crate::get_crate_identity(), Span::call_site());

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
    let carrier = |modified: &ItemTrait| {
        quote! {
            #trait_item

            #[allow(unused_macros, unused_imports, dead_code, non_local_definitions)]
            #[doc(hidden)]
            #[macro_export]
            macro_rules! #temporal_mac_name {
                (#crate_identity #crate_version [$_:path, $wl1:path $(,$wl:path)* $(,)?] {$($trait_defs:tt)*} $($t:tt)*) => {
                    $wl1! {
                        #crate_identity
                        #crate_version
                        [$wl1 $(,$wl)*]
                        {
                            #(for attr in &modified.attrs) { #attr }
                            #{&modified.vis}
                            #{&modified.unsafety}
                            #{&modified.auto_token}
                            #{&modified.trait_token}
                            #{&modified.ident}
                            #{&modified.generics}
                            #{&modified.colon_token}
                            #{&modified.supertraits}
                            {
                                #(for item in &modified.items) { #item }
                            },
                            $($trait_defs)*
                        }
                        $($t)*
                    }
                };
            }

            #(if alter_macro_name.is_none()) {
                #[doc(hidden)]
                #[allow(unused_imports, unused_macros, dead_code)]
                #{&trait_item.vis} use #temporal_mac_name as #{&trait_item.ident};
            } #(else) {
                #[doc(hidden)]
                #[allow(unused_imports, unused_macros, dead_code)]
                pub use #temporal_mac_name;
            }
        }
    };
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

    // Built AFTER the interning rewrite, so the definition that travels through the macro is the
    // rewritten one.
    let output0 = carrier(&modified_trait_item);

    quote! {
        #output0

        #typeref_impls
    }
}
