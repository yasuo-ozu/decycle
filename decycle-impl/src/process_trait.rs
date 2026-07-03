use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use proc_macro_error::*;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;
use syn::*;
use template_quote::quote;
use type_leak::Leaker;

pub fn process_trait(
    trait_item: &ItemTrait,
    decycle_path: &Path,
    marker_path: Option<&Path>,
    alter_macro_name: Option<&Ident>,
    leaker_config: type_leak::LeakerConfig,
) -> TokenStream2 {
    let random_suffix = crate::get_random();
    let temporal_mac_name = alter_macro_name.cloned().unwrap_or_else(|| {
        // `random_suffix` is deterministic (hashed from the fixed crate identity string),
        // so two `#[decycle] trait Foo` items in one crate (different modules) would
        // otherwise compute the exact same `#[macro_export]` name here and collide
        // (E0428). Fold a per-item discriminant — hashed from the trait's own tokens —
        // into THIS name only; `random_suffix` itself must stay untouched everywhere
        // else, since it's the key the leaker's `Repeater` impls (built independently,
        // elsewhere in this fn) are pinned to.
        let discriminant = crate::identity_to_u64(&quote!(#trait_item).to_string());
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
    let output0 = quote! {
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
                        #(for attr in &modified_trait_item.attrs) { #attr }
                        #{&modified_trait_item.vis}
                        #{&modified_trait_item.unsafety}
                        #{&modified_trait_item.auto_token}
                        #{&modified_trait_item.trait_token}
                        #{&modified_trait_item.ident}
                        #{&modified_trait_item.generics}
                        #{&modified_trait_item.colon_token}
                        #{&modified_trait_item.supertraits}
                        {
                            #(for item in &modified_trait_item.items) { #item }
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
    };
    proc_macro_error::set_dummy(output0.clone());

    let mut leaker = Leaker::from_config(leaker_config);
    leaker
        .intern_with(&trait_item.generics, |v| {
            v.visit_item_trait(trait_item);
        })
        .unwrap_or_else(|type_leak::NotInternableError(span)| abort!(span, "use absolute path"));
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
        #output0

        #typeref_impls
    }
}
