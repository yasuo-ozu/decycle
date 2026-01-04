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

    let mut leaker = Leaker::from_trait(trait_item)
        .unwrap_or_else(|type_leak::NotInternableError(span)| abort!(span, "use absolute path"));
    leaker.reduce_roots();
    let referrer = leaker.finish();
    let mut modified_trait_item = trait_item.clone();
    if !referrer.is_empty() {
        let marker_path = marker_path.unwrap_or_else(|| {
            abort!(
                Span::call_site(), "specify 'marker' arg";
                hint = referrer.iter().next().unwrap().span() => "first type to be interned"
            )
        });
        referrer
            .clone()
            .into_visitor(
                |ty, num| parse_quote!(<#marker_path as #decycle_path::Repeater<#num, #ty>>::Type),
            )
            .visit_item_trait_mut(&mut modified_trait_item);
    }
    // TODO: check that all trait path is absolute

    quote! {
        #trait_item

        #[allow(unused_macros, unused_imports, dead_code, non_local_definitions)]
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #temporal_mac_name {
            (#crate_version [$wl0:path $(,$wl:path)* $(,)?] {$($trait_defs:tt)*} {$($referrers:tt)*} {$($contents:tt)*}) => {
                $wl0! {
                    #crate_version
                    [$($wl),*]
                    { #modified_trait_item, $($trait_defs)* }
                    { $($contents)* }
                }
            };
        }

        #[doc(hidden)]
        #[allow(unused_imports, unused_macros, dead_code)]
        #{&trait_item.vis} use #temporal_mac_name as #{&trait_item.ident};

        #(for (num, ty) in referrer.iter().enumerate()) {
            impl #decycle_path::Repeater<#num> for #marker_path {
                type Type = #ty;
            }
        }
    }
}
