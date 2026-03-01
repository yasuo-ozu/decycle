use crate::helper::*;
use proc_macro2::{Span, TokenStream};
use proc_macro_error::*;
use std::collections::HashMap;
use std::sync::OnceLock;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::*;
use template_quote::quote;

macro_rules! parse_quote {
    ($($tt:tt)*) => {
        syn::parse2(::template_quote::quote!($($tt)*)).unwrap()
    };
}

macro_rules! name {
    ($($arg:tt)*) => {
        name(&format!($($arg)*))
    };
}

fn name(s: &str) -> Ident {
    static RANDOM_SUFFIX: OnceLock<String> = OnceLock::new();

    let suffix = RANDOM_SUFFIX.get_or_init(|| crate::get_random().to_string());
    Ident::new(&format!("{}{}", s, suffix), Span::call_site())
}

fn remove_cyclic_bounds(
    generics: &Generics,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> Generics {
    let mut g = generics.clone();
    replace_constraints(&mut g, |ty, trait_path| {
        (trait_path.segments.len() == 1
            && replacing_table
                .get(&trait_path.segments.last().unwrap().ident)
                .is_none())
        .then_some((ty, trait_path))
    });
    g
}

/// Check if a type's arguments contain any cycle participant types.
/// Returns true if the type itself is not a cycle participant but contains
/// cycle participants in its type arguments (e.g., `Box<B<S>>` where `B` is a
/// cycle participant).
fn type_contains_cycle_participant(
    ty: &Type,
    self_type_idents: &std::collections::HashSet<String>,
) -> bool {
    !find_cycle_participants_in_type(ty, self_type_idents).is_empty()
}

/// Find all cycle participant types within a type's arguments.
/// Returns the full type paths of cycle participants found (e.g., `B<S>` from `Box<B<S>>`).
fn find_cycle_participants_in_type(
    ty: &Type,
    self_type_idents: &std::collections::HashSet<String>,
) -> Vec<Type> {
    use syn::visit::Visit;
    struct CycleParticipantCollector<'a> {
        self_type_idents: &'a std::collections::HashSet<String>,
        found: Vec<Type>,
    }
    impl<'ast, 'a> Visit<'ast> for CycleParticipantCollector<'a> {
        fn visit_type_path(&mut self, tp: &'ast TypePath) {
            if tp.qself.is_none() {
                if let Some(last_seg) = tp.path.segments.last() {
                    if self.self_type_idents.contains(&last_seg.ident.to_string()) {
                        self.found.push(Type::Path(tp.clone()));
                        return;
                    }
                }
                if tp.path.is_ident("Self") {
                    self.found.push(Type::Path(tp.clone()));
                    return;
                }
            }
            syn::visit::visit_type_path(self, tp);
        }
    }
    let mut collector = CycleParticipantCollector {
        self_type_idents,
        found: Vec::new(),
    };
    // Only visit the type arguments, not the root type itself
    if let Type::Path(tp) = ty {
        for seg in &tp.path.segments {
            if let PathArguments::AngleBracketed(args) = &seg.arguments {
                for arg in &args.args {
                    collector.visit_generic_argument(arg);
                }
            }
        }
    }
    collector.found
}

/// Remove bounds on decycled traits where the bounded type is a wrapper
/// containing cycle participants (e.g., `Box<B<S>>: Parse<Atom>`).
/// These bounds create cycles through final impls that reset rank.
fn remove_wrapper_cycle_bounds(
    generics: &Generics,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> Generics {
    let self_type_idents: std::collections::HashSet<String> = replacing_table
        .values()
        .flat_map(|(_, _, impls)| impls.iter())
        .filter_map(|impl_| {
            if let Type::Path(tp) = impl_.self_ty.as_ref() {
                tp.path.segments.last().map(|s| s.ident.to_string())
            } else {
                None
            }
        })
        .collect();

    let mut g = generics.clone();
    replace_constraints(&mut g, |ty, trait_path| {
        if trait_path.segments.len() != 1 {
            return Some((ty, trait_path));
        }
        if replacing_table
            .get(&trait_path.segments.last().unwrap().ident)
            .is_some()
        {
            if let Type::Path(tp) = &ty {
                if !tp.path.is_ident("Self") {
                    let is_self_type = tp
                        .path
                        .segments
                        .last()
                        .map_or(false, |s| self_type_idents.contains(&s.ident.to_string()));
                    if !is_self_type && type_contains_cycle_participant(&ty, &self_type_idents) {
                        return None;
                    }
                }
            }
        }
        Some((ty, trait_path))
    });
    g
}

fn wrap_cyclic_bounds(
    generics: &Generics,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> Generics {
    // Collect self type root idents from all decycled impls — only these
    // concrete types participate in cycles and need wrapping.
    let self_type_idents: std::collections::HashSet<String> = replacing_table
        .values()
        .flat_map(|(_, _, impls)| impls.iter())
        .filter_map(|impl_| {
            if let Type::Path(tp) = impl_.self_ty.as_ref() {
                tp.path.segments.last().map(|s| s.ident.to_string())
            } else {
                None
            }
        })
        .collect();

    let mut g = generics.clone();
    replace_constraints(&mut g, |ty, trait_path| {
        if trait_path.segments.len() != 1 {
            return Some((ty, trait_path));
        }
        let last_ident = &trait_path.segments.last().unwrap().ident;
        if let Some((trait_, ix, _)) = replacing_table.get(last_ident) {
            // // Only wrap bounds on path types that could have decycled impls.
            // // Non-path types ((), tuples, references, etc.) cannot participate
            // // in trait cycles, so keep their bounds as-is.
            // if !matches!(&ty, Type::Path(_)) {
            //     return Some((ty, trait_path));
            // }

            // Only wrap types that directly participate in cycles:
            // - Self keyword (refers to the impl's self type)
            // - Concrete types matching a self type of a decycled impl
            // External types (e.g. Ident) and generic type parameters
            // don't have ranked impls for Wrapper<T>.
            // let ranked_trait_flag = if let Type::Path(tp) = &ty {
            //     if tp.path.is_ident("Self") {
            //         true
            //     } else if let Some(last_seg) = tp.path.segments.last() {
            //         replacing_table.contains_key(&last_seg.ident)
            //     } else {
            //         replacing_table.contains_key(&last_seg.ident)
            //     }
            // };

            // if should_wrap {
            return Some((
                parse_quote!(#{name!("Wrapper")}<#ty>),
                parse_quote!(
                    #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                    #{trait_path.segments.last().unwrap().arguments.insert(*ix, parse_quote![()]).insert(*ix, parse_quote![#{name!("Rank")}])}
                ),
            ));
            // }

            // For wrapper types containing cycle participants in their type
            // arguments (e.g., `Box<B<S>>: Parse<Atom>`): remove the bound
            // from the inductive step's WHERE clause. These bounds would
            // create cycles through final impls (which reset rank to
            // InitialRank). The mock impl uses the original WHERE clause
            // and can satisfy these through the final impls' ranked chains.
            if type_contains_cycle_participant(&ty, &self_type_idents) {
                return None;
            }

            Some((ty, trait_path))
        } else {
            Some((ty, trait_path))
        }
    });
    g
}

/// Collect non-cycle-participant bounds from all impls of a given trait.
/// These bounds are needed transitively when the mock impl's cycle-participant
/// bounds are proved through the final impl → ranked chain. By adding them
/// to the outer inductive step's where clause, they become available in the
/// param_env for nested trait resolution.
fn collect_transitive_bounds(
    trait_ident: &Ident,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> Vec<WherePredicate> {
    let self_type_idents: std::collections::HashSet<String> = replacing_table
        .values()
        .flat_map(|(_, _, impls)| impls.iter())
        .filter_map(|impl_| {
            if let Type::Path(tp) = impl_.self_ty.as_ref() {
                tp.path.segments.last().map(|s| s.ident.to_string())
            } else {
                None
            }
        })
        .collect();

    let mut bounds = Vec::new();

    if let Some((_, _, impls)) = replacing_table.get(trait_ident) {
        for impl_ in impls {
            // Collect from generic param bounds (e.g., `impl<K: Hash + Eq, ...>`)
            for param in &impl_.generics.params {
                if let GenericParam::Type(tp) = param {
                    let is_participant = self_type_idents.contains(&tp.ident.to_string());
                    if !is_participant && !tp.bounds.is_empty() {
                        bounds.push(WherePredicate::Type(PredicateType {
                            lifetimes: None,
                            bounded_ty: Type::Path(TypePath {
                                qself: None,
                                path: tp.ident.clone().into(),
                            }),
                            colon_token: Default::default(),
                            bounds: tp.bounds.clone(),
                        }));
                    }
                }
            }

            // Collect from where clause predicates
            if let Some(wc) = &impl_.generics.where_clause {
                for pred in &wc.predicates {
                    if let WherePredicate::Type(pt) = pred {
                        let bounded_ty = &pt.bounded_ty;

                        // Skip if bounded type is a cycle participant
                        let is_participant = if let Type::Path(tp) = bounded_ty {
                            tp.path.is_ident("Self")
                                || tp.path.segments.last().map_or(false, |s| {
                                    self_type_idents.contains(&s.ident.to_string())
                                })
                        } else {
                            false
                        };

                        // Skip if bounded type contains cycle participants
                        let contains_participant =
                            type_contains_cycle_participant(bounded_ty, &self_type_idents);

                        if !is_participant && !contains_participant {
                            bounds.push(pred.clone());
                        }
                    }
                }
            }
        }
    }

    bounds
}

fn emit_impl_items_leaf(impl_: &ItemImpl) -> TokenStream {
    let mut output = TokenStream::new();

    for (_ix, item) in impl_.items.iter().enumerate() {
        match item {
            ImplItem::Fn(ImplItemFn {
                defaultness, sig, ..
            }) => {
                let mut sig = sig.clone();
                // Don't replace Self — the leaf impl is for Wrapper<SelfTy>,
                // so Self should resolve to Wrapper<SelfTy> to match the ranked trait.
                // Only desugar impl Trait (pass Self as replacement = no-op for Self).
                replace_self_and_desugar_impl_trait(&mut sig, &parse_quote!(Self));

                for (ix, input) in sig.inputs.iter_mut().enumerate() {
                    input.reduce_pat(ix);
                }
                output.extend(quote! {
                    #defaultness #sig {
                        ::core::unimplemented!("decycle: cycle limit reached")
                    }
                });
            }
            o => output.extend(quote!(#o)),
        }
    }
    output
}

fn emit_trait_items_delegate(
    trait_: &ItemTrait,
    path: TokenStream,
    base_self_ty: &Type,
) -> TokenStream {
    let mut output = TokenStream::new();

    for item in &trait_.items {
        match item {
            TraitItem::Fn(TraitItemFn { sig, .. }) => {
                let mut sig = sig.clone();
                replace_self_and_desugar_impl_trait(&mut sig, base_self_ty);

                for (ix, input) in sig.inputs.iter_mut().enumerate() {
                    input.reduce_pat(ix);
                }
                output.extend(quote! {
                    #sig {
                        #path::#{&sig.ident}(
                            #(for input in &sig.inputs), {
                                #{input.variable()}
                            }
                        )
                    }
                })
            }
            TraitItem::Type(TraitItemType {
                ident, generics, ..
            }) => output.extend(quote! {
                type #ident #generics = #path::#ident;
            }),
            TraitItem::Const(TraitItemConst { ident, ty, .. }) => output.extend(quote! {
                const #ident: #ty = #path::#ident;
            }),
            _ => unimplemented!(),
        }
    }
    output
}

fn emit_impl_items_delegate(impl_: &ItemImpl, path: TokenStream) -> TokenStream {
    let mut output = TokenStream::new();
    for item in &impl_.items {
        match item {
            ImplItem::Fn(ImplItemFn {
                sig, defaultness, ..
            }) => {
                let mut sig = sig.clone();
                for (ix, input) in sig.inputs.iter_mut().enumerate() {
                    input.reduce_pat(ix);
                }
                output.extend(quote! {
                    #defaultness #sig {
                        #path::#{&sig.ident}(
                            #(for input in &sig.inputs), {
                                #{input.variable()}
                            }
                        )
                    }
                })
            }
            ImplItem::Type(ImplItemType {
                ident, generics, ..
            }) => output.extend(quote! {
                type #ident #generics = #path::#ident;
            }),
            ImplItem::Const(ImplItemConst { ident, ty, .. }) => output.extend(quote! {
                const #ident: #ty = #path::#ident;
            }),
            _ => unimplemented!(),
        }
    }
    output
}

/// Like `emit_impl_items_delegate` but delegates through `Wrapper<Self>`,
/// using pointer casts since `Wrapper` is `#[repr(transparent)]`.
fn emit_impl_items_delegate_via_wrapper(
    trait_: &ItemTrait,
    impl_: &ItemImpl,
    path: TokenStream,
) -> TokenStream {
    let mut output = TokenStream::new();
    for item in &impl_.items {
        match item {
            ImplItem::Fn(ImplItemFn {
                sig, defaultness, ..
            }) => {
                // Find corresponding trait method to check Self in types
                let trait_item_fn = trait_.items.iter().find_map(|ti| match ti {
                    TraitItem::Fn(tif) if tif.sig.ident == sig.ident => Some(tif),
                    _ => None,
                });
                let trait_inputs: Vec<_> = trait_item_fn
                    .map(|tif| tif.sig.inputs.iter().collect())
                    .unwrap_or_default();

                let mut sig = sig.clone();
                let wrapper_name = name!("Wrapper");
                let args = sig
                    .inputs
                    .iter_mut()
                    .enumerate()
                    .map(|(ix, input)| {
                        input.reduce_pat(ix);
                        match input {
                            FnArg::Receiver(Receiver {
                                reference,
                                mutability,
                                self_token,
                                ty,
                                ..
                            }) => {
                                // Wrap Self → Wrapper<Self> via repr(transparent) pointer cast
                                if reference.is_some() {
                                    if mutability.is_some() {
                                        quote!(unsafe { &mut *(#self_token as *mut Self as *mut #wrapper_name<Self>) })
                                    } else {
                                        quote!(unsafe { &*(#self_token as *const Self as *const #wrapper_name<Self>) })
                                    }
                                } else if is_plain_self_type(ty) {
                                    quote!(#wrapper_name(#self_token))
                                } else {
                                    // Complex receiver like Box<Self>: use ptr::read + forget
                                    let wrapped_ty: Type = replace_self_in_type(ty, &parse_quote!(#wrapper_name<Self>));
                                    quote!(unsafe {
                                        let __raw = &#self_token as *const _ as *const #wrapped_ty;
                                        let __result = ::core::ptr::read(__raw);
                                        ::core::mem::forget(#self_token);
                                        __result
                                    })
                                }
                            }
                            FnArg::Typed(PatType { pat, .. }) => {
                                // Check trait's corresponding arg for Self
                                if let Some(FnArg::Typed(PatType { ty: trait_ty, .. })) =
                                    trait_inputs.get(ix).copied()
                                {
                                    if type_contains_self(trait_ty) {
                                        quote!(unsafe { &*(#pat as *const _ as *const #wrapper_name<Self>) })
                                    } else {
                                        quote!(#pat)
                                    }
                                } else {
                                    quote!(#pat)
                                }
                            }
                        }
                    })
                    .collect::<Punctuated<_, Token![,]>>();

                // Check if return type contains Self
                let return_contains_self = trait_item_fn
                    .map(|tif| match &tif.sig.output {
                        ReturnType::Default => false,
                        ReturnType::Type(_, ty) => type_contains_self(ty),
                    })
                    .unwrap_or(false);

                let call = quote!(#path::#{&sig.ident}(#args));
                // When the return type contains Self, the ranked method returns
                // with Self=Wrapper<SelfTy> but we need Self=SelfTy.
                // Since Wrapper is repr(transparent), they have identical
                // layout, so we reinterpret via ptr::read + forget.
                let body = if return_contains_self {
                    quote!({
                        let __ranked_result = unsafe { #call };
                        unsafe {
                            let __raw = &__ranked_result as *const _ as *const _;
                            let __converted = ::core::ptr::read(__raw);
                            ::core::mem::forget(__ranked_result);
                            __converted
                        }
                    })
                } else {
                    quote!(unsafe { #call })
                };

                output.extend(quote! {
                    #defaultness #sig {
                        #body
                    }
                })
            }
            ImplItem::Type(ImplItemType {
                ident, generics, ..
            }) => output.extend(quote! {
                type #ident #generics = #path::#ident;
            }),
            ImplItem::Const(ImplItemConst { ident, ty, .. }) => output.extend(quote! {
                const #ident: #ty = #path::#ident;
            }),
            _ => unimplemented!(),
        }
    }
    output
}

fn emit_impl_main(trait_: &ItemTrait, impl_: &ItemImpl, base_self_ty: &Type) -> TokenStream {
    let mut output = TokenStream::new();
    for item in &impl_.items {
        match item {
            ImplItem::Fn(impl_item_fn) => {
                if let Some(trait_item_fn) = trait_.items.iter().find_map(|item| match item {
                    TraitItem::Fn(trait_item_fn)
                        if &trait_item_fn.sig.ident == &impl_item_fn.sig.ident =>
                    {
                        Some(trait_item_fn)
                    }
                    _ => None,
                }) {
                    let impl_mock_path = quote!(
                        #{name!("{}Mock", &trait_.ident)}
                        #{&impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments}
                    );
                    // Don't replace Self in outer sig — the inductive step impl is
                    // for Wrapper<SelfTy>, so Self = Wrapper<SelfTy>.
                    let mut sig_outer = impl_item_fn.sig.clone();
                    replace_self_and_desugar_impl_trait(&mut sig_outer, &parse_quote!(Self));

                    // Build args: convert from Wrapper<SelfTy> to SelfTy for mock call
                    let trait_inputs: Vec<_> = trait_item_fn.sig.inputs.iter().collect();
                    let args = sig_outer
                        .inputs
                        .iter_mut()
                        .enumerate()
                        .map(|(ix, input)| {
                            input.reduce_pat(ix);
                            match input {
                                FnArg::Receiver(Receiver {
                                    reference,
                                    mutability,
                                    self_token,
                                    ty,
                                    ..
                                }) => {
                                    // Unwrap Wrapper<SelfTy> → SelfTy
                                    if reference.is_some() {
                                        if mutability.is_some() {
                                            quote!(&mut #self_token.0)
                                        } else {
                                            quote!(&#self_token.0)
                                        }
                                    } else if is_plain_self_type(ty) {
                                        quote!(#self_token.0)
                                    } else {
                                        // Complex receiver like Box<Self>: use ptr::read + forget
                                        let unwrapped_ty = replace_self_in_type(ty, base_self_ty);
                                        quote!(unsafe {
                                            let __raw = &#self_token as *const _ as *const #unwrapped_ty;
                                            let __result = ::core::ptr::read(__raw);
                                            ::core::mem::forget(#self_token);
                                            __result
                                        })
                                    }
                                }
                                FnArg::Typed(PatType { pat, .. }) => {
                                    // Check trait's corresponding arg for Self
                                    if let Some(FnArg::Typed(PatType { ty: trait_ty, .. })) =
                                        trait_inputs.get(ix).copied()
                                    {
                                        if type_contains_self(trait_ty) {
                                            // Convert Wrapper-based to bare via .0
                                            quote!(unsafe { &*(#pat as *const _ as *const #base_self_ty) })
                                        } else {
                                            quote!(#pat)
                                        }
                                    } else {
                                        quote!(#pat)
                                    }
                                }
                            }
                        })
                        .collect::<Punctuated<_, Token![,]>>();

                    // Check if return type contains Self
                    let return_contains_self = match &trait_item_fn.sig.output {
                        ReturnType::Default => false,
                        ReturnType::Type(_, ty) => type_contains_self(ty),
                    };

                    // let call = quote!(
                    //     <#base_self_ty as #impl_mock_path>::#{name!("{}_mocked_", &impl_item_fn.sig.ident)}(#args)
                    // );

                    let call = quote!(
                        <#{&impl_.self_ty} as #{&impl_.trait_.as_ref().unwrap().1}>::#{&impl_item_fn.sig.ident}(#args)
                    );

                    // When the return type contains Self, the mock returns with
                    // Self=base_self_ty but we need Self=Wrapper<base_self_ty>.
                    // Since Wrapper is repr(transparent), they have identical
                    // layout, so we reinterpret via ptr::read + forget.
                    let body = if return_contains_self {
                        quote!({
                            let __mock_result = #call;
                            unsafe {
                                let __raw = &__mock_result as *const _ as *const _;
                                let __converted = ::core::ptr::read(__raw);
                                ::core::mem::forget(__mock_result);
                                __converted
                            }
                        })
                    } else {
                        call
                    };

                    output.extend(quote!(
                        #{&impl_item_fn.defaultness} #{&sig_outer}{
                            #body
                        }
                    ));

                    // output.extend(quote!(
                    //     #{&impl_item_fn.defaultness} #{&sig_outer}{
                    //         trait #{name!("{}Mock", &trait_.ident)} #{trait_.generics.impl_generics()} {
                    //             #(for item in &trait_.items) {
                    //                 #(if !matches!(item, TraitItem::Fn(_))) {
                    //                     #item
                    //                 }
                    //             }
                    //             #{
                    //                 let mut ti = trait_item_fn.clone();
                    //                 ti.sig.ident = name!("{}_mocked_", &ti.sig.ident);
                    //                 ti.default = None;
                    //                 ti.semi_token = Default::default();
                    //                 ti
                    //             }
                    //         }
                    //         impl #{impl_.generics.impl_generics()} #impl_mock_path for #base_self_ty
                    //         #{&impl_.generics.where_clause} {
                    //             #(for item in &impl_.items) {
                    //                 #(if !matches!(item, ImplItem::Fn(_))) {
                    //                     #item
                    //                 }
                    //             }
                    //             #{
                    //                 let mut ii = impl_item_fn.clone();
                    //                 ii.sig.ident = name!("{}_mocked_", &ii.sig.ident);
                    //                 ii
                    //             }
                    //         }
                    //         #body
                    //     }
                    // ));
                } else {
                    output.extend(quote!(#item));
                }
            }
            o => output.extend(quote!(#o)),
        }
    }
    output
}

fn type_contains_self(ty: &Type) -> bool {
    use syn::visit::Visit;
    struct SelfFinder {
        found: bool,
    }
    impl<'ast> Visit<'ast> for SelfFinder {
        fn visit_type_path(&mut self, tp: &'ast TypePath) {
            if tp.qself.is_none() && tp.path.is_ident("Self") {
                self.found = true;
                return;
            }
            // Don't recurse into qualified paths like Self::Error —
            // associated types resolve to the same concrete type regardless
            // of wrapping, so they don't need conversion.
            if tp.qself.is_some() {
                return;
            }
            syn::visit::visit_type_path(self, tp);
        }
    }
    let mut finder = SelfFinder { found: false };
    finder.visit_type(ty);
    finder.found
}

fn is_plain_self_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(TypePath { qself: None, path }) if path.is_ident("Self"))
}

fn replace_self_in_type(ty: &Type, replacement: &Type) -> Type {
    use syn::visit_mut::VisitMut;
    struct SelfReplacer<'a> {
        replacement: &'a Type,
    }
    impl<'a> VisitMut for SelfReplacer<'a> {
        fn visit_type_mut(&mut self, ty: &mut Type) {
            if let Type::Path(TypePath { qself: None, path }) = ty {
                if path.is_ident("Self") {
                    *ty = self.replacement.clone();
                    return;
                }
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
    let mut ty = ty.clone();
    SelfReplacer { replacement }.visit_type_mut(&mut ty);
    ty
}

fn replace_self(sig: &mut Signature, base_self_ty: &Type) {
    use syn::visit_mut::VisitMut;

    struct SelfReplacer<'a> {
        base_self_ty: &'a Type,
    }

    impl<'a> VisitMut for SelfReplacer<'a> {
        fn visit_receiver_mut(&mut self, _receiver: &mut Receiver) {
            // Skip visiting receiver to avoid replacing Self in receiver types
        }

        fn visit_type_mut(&mut self, ty: &mut Type) {
            if let Type::Path(TypePath { qself: None, path }) = ty {
                if path.is_ident("Self") {
                    *ty = self.base_self_ty.clone();
                    return;
                }
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }

    let mut replacer = SelfReplacer { base_self_ty };
    replacer.visit_signature_mut(sig);
}

fn replace_self_and_desugar_impl_trait(sig: &mut Signature, base_self_ty: &Type) {
    replace_self(sig, base_self_ty);

    let mut param_counter = 0usize;

    // Replace input-position impl Trait with type parameters
    for input in &mut sig.inputs {
        if let FnArg::Typed(PatType { ty, .. }) = input {
            if let Type::ImplTrait(impl_trait) = &**ty {
                let param_name = name!("ImplTrait{}", param_counter);
                param_counter += 1;

                let bounds = impl_trait.bounds.clone();
                sig.generics.params.push(GenericParam::Type(TypeParam {
                    attrs: Vec::new(),
                    ident: param_name.clone(),
                    colon_token: if bounds.is_empty() {
                        None
                    } else {
                        Some(Default::default())
                    },
                    bounds,
                    eq_token: None,
                    default: None,
                }));

                *ty = Box::new(Type::Path(TypePath {
                    qself: None,
                    path: param_name.into(),
                }));
            }
        }
    }
}

fn process_trait_item_for_ranked(item: &TraitItem) -> TraitItem {
    let mut item = item.clone();
    if let TraitItem::Fn(TraitItemFn {
        sig,
        default,
        semi_token,
        ..
    }) = &mut item
    {
        replace_self_and_desugar_impl_trait(sig, &parse_quote!(Self));
        *default = None;
        *semi_token = Some(Default::default());
    }
    item
}

fn parse_comma_separated<T: Parse>(input: ParseStream) -> Result<Vec<T>> {
    let mut items = Vec::new();
    while !input.is_empty() {
        items.push(input.parse()?);
        if !input.is_empty() {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(items)
}

/// Returns `false` for trait bounds whose path is a single segment present
/// in `replacing_table`, used to filter out bounds that will be replaced.
#[allow(dead_code)]
fn should_keep_bound(
    bound: &TypeParamBound,
    replacing_table: &HashMap<Ident, (usize, Path)>,
) -> bool {
    if let TypeParamBound::Trait(trait_bound) = bound {
        if trait_bound.path.segments.len() == 1 {
            return !replacing_table.contains_key(&trait_bound.path.segments[0].ident);
        }
    }
    true
}

/// Strips bounds matching `replacing_table` from a `Generics`, removing
/// type param bounds and where-clause predicates whose paths appear as keys.
#[allow(dead_code)]
fn strip_replaced_bounds(generics: &mut Generics, replacing_table: &HashMap<Ident, (usize, Path)>) {
    for param in &mut generics.params {
        if let GenericParam::Type(ref mut type_param) = param {
            type_param.bounds = type_param
                .bounds
                .iter()
                .filter(|bound| should_keep_bound(bound, replacing_table))
                .cloned()
                .collect();
            if type_param.bounds.is_empty() {
                type_param.colon_token = None;
            }
        }
    }
    if let Some(ref mut where_clause) = generics.where_clause {
        where_clause.predicates = where_clause
            .predicates
            .iter()
            .filter_map(|pred| {
                if let WherePredicate::Type(type_pred) = pred {
                    let new_bounds: Punctuated<TypeParamBound, Token![+]> = type_pred
                        .bounds
                        .iter()
                        .filter(|bound| should_keep_bound(bound, replacing_table))
                        .cloned()
                        .collect();
                    if new_bounds.is_empty() {
                        None
                    } else {
                        let mut new_pred = type_pred.clone();
                        new_pred.bounds = new_bounds;
                        Some(WherePredicate::Type(new_pred))
                    }
                } else {
                    Some(pred.clone())
                }
            })
            .collect();
        if where_clause.predicates.is_empty() {
            generics.where_clause = None;
        }
    }
}

pub struct FinalizeArgs {
    pub working_list: Vec<Path>,
    pub traits: Vec<ItemTrait>,
    pub contents: Vec<ItemImpl>,
    pub recurse_level: usize,
}

impl Parse for FinalizeArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let _crate_identity: LitStr = input.parse()?;
        let crate_version: LitStr = input.parse()?;
        let expected_version = env!("CARGO_PKG_VERSION");
        if crate_version.value() != expected_version {
            abort!(
                Span::call_site(),
                "version mismatch: expected '{}', got '{}'",
                expected_version,
                crate_version.value()
            )
        }

        let working_list_content;
        bracketed!(working_list_content in input);
        let working_list = parse_comma_separated(&working_list_content)?;

        let traits_content;
        braced!(traits_content in input);
        let traits = parse_comma_separated(&traits_content)?;

        let contents_content;
        braced!(contents_content in input);
        let contents = parse_comma_separated(&contents_content)?;

        let lit: LitInt = input.parse()?;
        let recurse_level = lit.base10_parse()?;

        Ok(FinalizeArgs {
            working_list,
            traits,
            contents,
            recurse_level,
        })
    }
}

impl template_quote::ToTokens for FinalizeArgs {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let crate_identity = LitStr::new(&crate::get_crate_identity(), Span::call_site());
        let crate_version = env!("CARGO_PKG_VERSION");
        let working_list = &self.working_list;
        let traits = &self.traits;
        let contents = &self.contents;

        let recurse_level = &self.recurse_level;

        tokens.extend(quote! {
            #crate_identity
            #crate_version
            [ #(#working_list),* ]
            { #(#traits),* }
            { #(#contents),* }
            #recurse_level
        });
    }
}

fn get_initial_rank(count: usize) -> Type {
    if count == 0 {
        parse_quote!(())
    } else {
        let inner = get_initial_rank(count - 1);
        parse_quote!((#inner,))
    }
}

fn replace_constraints(
    generics: &mut Generics,
    mut f: impl FnMut(Type, Path) -> Option<(Type, Path)>,
) {
    let mut where_clause = WhereClause {
        where_token: Default::default(),
        predicates: Punctuated::new(),
    };
    let mut where_predicates_to_add = Vec::new();
    let mut process_type_param_bound =
        |param_type: &Type, bound: TypeParamBound| -> Option<TypeParamBound> {
            match bound {
                TypeParamBound::Trait(TraitBound {
                    paren_token,
                    modifier: TraitBoundModifier::None,
                    lifetimes,
                    path,
                }) => {
                    if let Some((new_ty, new_path)) = f(param_type.clone(), path.clone()) {
                        if new_ty == *param_type {
                            Some(TypeParamBound::Trait(TraitBound {
                                paren_token,
                                modifier: TraitBoundModifier::None,
                                lifetimes,
                                path,
                            }))
                        } else {
                            where_predicates_to_add.push(WherePredicate::Type(PredicateType {
                                lifetimes: None,
                                bounded_ty: new_ty,
                                colon_token: Default::default(),
                                bounds: std::iter::once(TypeParamBound::Trait(TraitBound {
                                    paren_token: None,
                                    modifier: TraitBoundModifier::None,
                                    lifetimes: None,
                                    path: new_path,
                                }))
                                .collect(),
                            }));
                            None
                        }
                    } else {
                        None
                    }
                }
                bound => Some(bound),
            }
        };
    for param in &mut generics.params {
        if let GenericParam::Type(ref mut type_param) = param {
            let param_type = Type::Path(TypePath {
                qself: None,
                path: type_param.ident.clone().into(),
            });

            type_param.bounds = std::mem::take(&mut type_param.bounds)
                .into_iter()
                .filter_map(|bound| process_type_param_bound(&param_type, bound))
                .collect();

            if type_param.bounds.is_empty() {
                type_param.colon_token = None;
            }
        }
    }

    for pred in std::mem::take(&mut generics.where_clause)
        .map(|wc| wc.predicates)
        .into_iter()
        .flatten()
    {
        match pred {
            WherePredicate::Type(PredicateType {
                lifetimes,
                bounded_ty,
                colon_token,
                mut bounds,
            }) => {
                bounds = bounds
                    .into_iter()
                    .filter_map(|bound| process_type_param_bound(&bounded_ty, bound))
                    .collect();

                if !bounds.is_empty() {
                    where_clause
                        .predicates
                        .push(WherePredicate::Type(PredicateType {
                            lifetimes,
                            bounded_ty,
                            colon_token,
                            bounds,
                        }));
                }
            }
            o => where_clause.predicates.push(o),
        }
    }

    where_clause.predicates.extend(where_predicates_to_add);

    generics.where_clause = (!where_clause.predicates.is_empty()).then_some(where_clause);
}

pub fn finalize(args: FinalizeArgs) -> TokenStream {
    let replacing_table: HashMap<Ident, (ItemTrait, usize, Vec<_>)> = args
        .traits
        .iter()
        .map(|trait_| {
            let g = &trait_.generics;
            let loc = g
                .params
                .iter()
                .position(|param| !matches!(param, GenericParam::Lifetime(_)))
                .unwrap_or(g.params.len());
            let impls = args
                .contents
                .iter()
                .filter(|item_impl| {
                    item_impl
                        .trait_
                        .as_ref()
                        .and_then(|p| p.1.segments.last())
                        .is_some_and(|seg| seg.ident == trait_.ident)
                })
                .cloned()
                .collect::<Vec<_>>();
            if impls.is_empty() {
                emit_warning!(
                    &trait_.ident,
                    "trait '{}' has no implementations",
                    &trait_.ident
                );
            }
            (trait_.ident.clone(), (trait_.clone(), loc, impls))
        })
        .collect();

    let _output = TokenStream::new();
    let initial_rank = get_initial_rank(args.recurse_level);

    quote! {
        // this module is to prevent confliction of trait method call between ranked and non-ranked
        // traits
        #[doc(hidden)]
        mod #{name!("shadowing_module")} {

            #[allow(unused)]
            #[repr(transparent)]
            pub struct #{name!("Wrapper")}<T: ?::core::marker::Sized>(T);

            // This should be `pub` to disturb "private associated type `MyTraitRanked::AssocTy` in public interface"
            // when deligating MyTrait
            pub mod #{name!("ranked_traits")} {

                // for ImplSelfTy
                #[allow(unused)]
                use super::{ super::*, #{name!("Wrapper")}};

                #(for (trait_, rank_loc, impls) in replacing_table.values()) {

                    // pub trait MyTraitRanked<'a, Ranked, Flag, T>
                    #[allow(unused)]
                    #[doc(hidden)]
                    pub trait #{name!("{}Ranked", &trait_.ident)}
                    #{trait_.generics.insert(*rank_loc, parse_quote!(#{name!("Flag")})).insert(*rank_loc, parse_quote!(#{name!("Rank")})).ty_generics()}
                    #{trait_.colon_token} #{&trait_.supertraits} {
                        #(for item in &trait_.items) { #{process_trait_item_for_ranked(item)} }
                    }

                    // impl <
                    //     'a, Rank, T, SelfTy: MyTrait<'a, T>
                    // > MyTraitRanked<'a, Rank, ((),), T> for SelfTy
                    impl #{trait_.generics.insert_last(parse_quote!(
                        #{name!("SelfTy")}: #{&trait_.ident} #{trait_.generics.ty_generics()}
                    )).insert_last(parse_quote!(#{name!("Rank")})).impl_generics()} #{name!("{}Ranked", &trait_.ident)}
                    #{trait_.generics.ty_generics().insert(*rank_loc, parse_quote![((),)]).insert(*rank_loc, parse_quote![#{name!("Rank")}])} for #{name!("SelfTy")} {
                        #{emit_trait_items_delegate(
                            trait_,
                            quote!(<Self as #{&trait_.ident} #{trait_.generics.ty_generics()}>),
                            &parse_quote!(Self)
                        )}
                    }

                    #(for impl_ in impls) {

                        #(let g = remove_cyclic_bounds(&impl_.generics, &replacing_table)) {
                            // impl<'a, T> MyTraitRanked<'a, (), (), T> for Wrapper<ImplSelfTy>
                            #[allow(unused_variables)]
                            impl #{impl_.generics.impl_generics()}
                            #{name!("{}Ranked", &trait_.ident)}
                            #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, parse_quote![()]).insert(*rank_loc, parse_quote![()])}
                            for #{name!("Wrapper")}<#{&impl_.self_ty}> #{&g.where_clause} {
                                #{emit_impl_items_leaf(impl_)}
                            }

                            // impl<'a, T> super::super::MyTrait<'a, T, ()> for ImplSelfTy
                            //  where Wrapper<Self>: MyTraitRanked<'a, ((((),),),), (), T>
                            #(for attr in &impl_.attrs) { #attr }
                            #{&impl_.defaultness} #{&impl_.unsafety} impl #{g.impl_generics()}
                            #{&trait_.ident}
                            #{&impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments}
                            for #{&impl_.self_ty} #{g.push_predicate(parse_quote!(
                                #{name!("Wrapper")}<Self>: #{name!("{}Ranked", &trait_.ident)}
                                #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, parse_quote![()]).insert(*rank_loc, initial_rank.clone())}
                            )).where_clause}
                            {
                                #{emit_impl_items_delegate_via_wrapper(trait_, impl_, quote!(
                                    <#{name!("Wrapper")}<Self> as #{name!("{}Ranked", &trait_.ident)}
                                    #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, parse_quote![()]).insert(*rank_loc, initial_rank.clone())} >
                                ))}
                            }
                        }

                    }
                }
            }

            #[allow(unused)]
            use super::*;

            #(for (trait_, rank_loc, impls) in replacing_table.values()) {
                #(for impl_ in impls) {

                    #[allow(unused)]
                    use super::super::*;

                    #(let g = {
                        let mut g = wrap_cyclic_bounds(&impl_.generics, &replacing_table);
                        // for bound in collect_transitive_bounds(&trait_.ident, &replacing_table) {
                        //     g = g.push_predicate(bound);
                        // }
                        g
                    }) {
                        // impl<'a, Rank, T> ranked_traits::MyTraitRanked<'a, (Rank,), (), T> for
                        // Wrapper<ImplSelfTy>
                        // where
                        //     Self: ranked_traits::MyTraitRanked<'a, Rank, (), T>
                        impl #{g.insert_last(parse_quote!(#{name!("Rank")})).impl_generics()}
                        #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                        #{impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments.insert(*rank_loc, parse_quote![()]).insert(*rank_loc, parse_quote!((#{name!("Rank")},)))}
                        for #{name!("Wrapper")}<#{impl_.self_ty.as_ref()}> #{g.push_predicate(parse_quote!(
                            Self: #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)} #{&impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments.insert(*rank_loc, parse_quote![()]).insert(*rank_loc, parse_quote!(#{name!("Rank")}))}
                        )).where_clause} {
                            #{emit_impl_main(trait_, impl_, &impl_.self_ty)}
                        }
                    }
                }
            }
        }
    }
}
