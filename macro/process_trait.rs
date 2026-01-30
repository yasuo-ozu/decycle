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
) -> TokenStream2 {
    let random_suffix = crate::get_random();
    let temporal_mac_name = syn::Ident::new(
        &format!("__{}_temporal_{}", &trait_item.ident, random_suffix),
        trait_item.ident.span(),
    );
    let crate_version = env!("CARGO_PKG_VERSION");

    let mut modified_trait_item = trait_item.clone();
    // Randomize Ident of GenericParam in modified_trait_item.generics
    let generic_renames: Vec<(Ident, Ident)> = modified_trait_item
        .generics
        .params
        .iter_mut()
        .filter_map(|param| {
            let ident = match param {
                GenericParam::Type(tp) => &mut tp.ident,
                GenericParam::Const(cp) => &mut cp.ident,
                GenericParam::Lifetime(_) => return None,
            };
            let old_ident = ident.clone();
            let new_ident = Ident::new(&format!("{}_{}", ident, random_suffix), ident.span());
            *ident = new_ident.clone();
            Some((old_ident, new_ident))
        })
        .collect();
    struct GenericRenamer<'a>(&'a [(Ident, Ident)]);
    impl VisitMut for GenericRenamer<'_> {
        fn visit_ident_mut(&mut self, ident: &mut Ident) {
            for (old, new) in self.0 {
                if ident == old {
                    *ident = new.clone();
                    return;
                }
            }
        }
    }
    GenericRenamer(&generic_renames).visit_item_trait_mut(&mut modified_trait_item);
    let output0 = quote! {
        #trait_item

        #[allow(unused_macros, unused_imports, dead_code, non_local_definitions)]
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #temporal_mac_name {
            (#crate_version [$_:path, $wl1:path $(,$wl:path)* $(,)?] {$($trait_defs:tt)*} $($t:tt)*) => {
                $wl1! {
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

        #[doc(hidden)]
        #[allow(unused_imports, unused_macros, dead_code)]
        #{&trait_item.vis} use #temporal_mac_name as #{&trait_item.ident};
    };
    proc_macro_error::set_dummy(output0.clone());

    let mut leaker = Leaker::from_trait(trait_item)
        .unwrap_or_else(|type_leak::NotInternableError(span)| abort!(span, "use absolute path"));
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

    // Check that all trait paths are absolute in modified_trait_item
    {
        use syn::visit::Visit;
        struct AbsolutePathChecker<'a> {
            generic_params: Vec<&'a Ident>,
        }
        impl<'ast> Visit<'ast> for AbsolutePathChecker<'_> {
            fn visit_path(&mut self, path: &'ast Path) {
                // Skip if path has leading :: (absolute)
                if path.leading_colon.is_some() {
                    return;
                }
                // Skip if path is a single segment that matches a generic param
                if path.segments.len() == 1 {
                    let ident = &path.segments[0].ident;
                    // Skip Self
                    if ident == "Self" {
                        return;
                    }
                    // Skip generic parameters
                    if self.generic_params.iter().any(|p| *p == ident) {
                        return;
                    }
                }
                // For multi-segment paths without leading ::, check if first segment looks like a crate
                if path.segments.len() > 1 {
                    let first = &path.segments[0].ident;
                    // Common crate names that are allowed without ::
                    if first == "std"
                        || first == "core"
                        || first == "alloc"
                        || first == "crate"
                        || first == "super"
                        || first == "self"
                    {
                        syn::visit::visit_path(self, path);
                        return;
                    }
                    emit_warning!(
                        path,
                        "path '{}' may not be absolute; consider using '::' prefix",
                        quote!(#path)
                    );
                }
                syn::visit::visit_path(self, path);
            }
        }
        let generic_params: Vec<_> = modified_trait_item
            .generics
            .params
            .iter()
            .filter_map(|p| match p {
                GenericParam::Type(tp) => Some(&tp.ident),
                GenericParam::Const(cp) => Some(&cp.ident),
                GenericParam::Lifetime(_) => None,
            })
            .collect();
        AbsolutePathChecker { generic_params }.visit_item_trait(&modified_trait_item);
    }

    quote! {
        #output0

        #typeref_impls

        #(for (num, ty) in referrer.iter().enumerate()) {
            impl #decycle_path::Repeater<#num> for #marker_path {
                type Type = #ty;
            }
        }
    }
}
