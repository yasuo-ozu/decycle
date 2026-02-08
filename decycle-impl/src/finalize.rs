use crate::helper::*;
use proc_macro2::{Span, TokenStream};
use proc_macro_error::*;
use std::collections::HashMap;
use std::sync::OnceLock;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::visit_mut::VisitMut;
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

fn wrap_cyclic_bounds(
    generics: &Generics,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> Generics {
    let mut g = generics.clone();
    replace_constraints(&mut g, |ty, trait_path| {
        if trait_path.segments.len() != 1 {
            return Some((ty, trait_path));
        }
        if let Some((trait_, ix, _)) =
            replacing_table.get(&trait_path.segments.last().unwrap().ident)
        {
            Some((
                parse_quote!(#{name!("Wrapper")}<#ty>),
                parse_quote!(
                    #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                    #{trait_.generics.ty_generics().insert(*ix, parse_quote![#{name!("Rank")}])}
                ),
            ))
        } else {
            Some((ty, trait_path))
        }
    });
    g
}

fn emit_impl_items_leaf(impl_: &ItemImpl) -> TokenStream {
    let mut output = TokenStream::new();

    for (_ix, item) in impl_.items.iter().enumerate() {
        match item {
            ImplItem::Fn(ImplItemFn {
                defaultness, sig, ..
            }) => {
                let mut sig = sig.clone();
                replace_self_and_desugar_impl_trait(&mut sig, &impl_.self_ty);

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
                    let mut sig_with_reduced_pats = impl_item_fn.sig.clone();
                    replace_self(&mut sig_with_reduced_pats, base_self_ty);
                    let args = sig_with_reduced_pats
                        .inputs
                        .iter_mut()
                        .enumerate()
                        .map(|(ix, input)| {
                            input.reduce_pat(ix);
                            match input {
                                FnArg::Receiver(Receiver { self_token, .. }) => {
                                    quote!(::core::mem::transmute(#self_token))
                                }
                                FnArg::Typed(PatType { pat, .. }) => {
                                    quote!(#pat)
                                }
                            }
                        })
                        .collect::<Punctuated<_, Token![,]>>();

                    output.extend(quote!(
                        #{&impl_item_fn.defaultness} #{&sig_with_reduced_pats}{
                            trait #{name!("{}Mock", &trait_.ident)} #{trait_.generics.impl_generics()}: #{&trait_.ident} {
                                #{
                                    let mut ti = trait_item_fn.clone();
                                    ti.sig.ident = name!("{}_mocked_", &ti.sig.ident);
                                    ti.default = None;
                                    ti.semi_token = Default::default();
                                    ti
                                }
                            }
                            impl #{impl_.generics.impl_generics()} #impl_mock_path for #base_self_ty
                            #{&impl_.generics.where_clause} {
                                #{
                                    let mut ii = impl_item_fn.clone();
                                    ii.sig.ident = name!("{}_mocked_", &ii.sig.ident);
                                    ii
                                }
                            }
                            unsafe {
                                <#base_self_ty as #impl_mock_path>::#{name!("{}_mocked_", &impl_item_fn.sig.ident)} ( #args )
                            }
                        }
                    ));
                } else {
                    output.extend(quote!(#item));
                }
            }
            o => output.extend(quote!(#o)),
        }
    }
    output
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

/// Replaces trait paths that have a single segment matching a key in the
/// HashMap with the corresponding replacement Path, copying the original
/// PathArguments and inserting the given Type at the stored position.
#[allow(dead_code)]
struct TraitReplacer(HashMap<Ident, (usize, Path)>, Type);

impl VisitMut for TraitReplacer {
    fn visit_expr_path_mut(&mut self, expr_path: &mut ExprPath) {
        self.replace_qself_path(expr_path.qself.as_mut(), &mut expr_path.path);
        syn::visit_mut::visit_expr_path_mut(self, expr_path);
    }

    fn visit_type_path_mut(&mut self, type_path: &mut TypePath) {
        self.replace_qself_path(type_path.qself.as_mut(), &mut type_path.path);
        syn::visit_mut::visit_type_path_mut(self, type_path);
    }

    fn visit_path_mut(&mut self, path: &mut Path) {
        self.replace_qself_path(None, path);
        syn::visit_mut::visit_path_mut(self, path);
    }
}

impl TraitReplacer {
    fn replace_qself_path(&mut self, qself: Option<&mut QSelf>, path: &mut Path) -> bool {
        // allow `Trait` or `<_ as Trait>::path`
        if !(matches!(qself, Some(QSelf { position: 1, .. })) || qself.is_none())
            || path.leading_colon.is_some()
        {
            return false;
        }

        if let Some((index, replacement)) = self.0.get(&path.segments[0].ident) {
            let orig_args = std::mem::replace(&mut path.segments[0].arguments, PathArguments::None);
            let mut new_path = replacement.clone();
            new_path.segments.last_mut().unwrap().arguments = orig_args;
            path_insert_type_arg(&mut new_path, *index, self.1.clone());
            let mut new_segments: Punctuated<PathSegment, Token![::]> = Punctuated::new();
            for seg in new_path.segments {
                new_segments.push(seg);
            }
            if let Some(qself) = qself {
                qself.position = new_segments.len();
                for seg in path.segments.iter().skip(qself.position) {
                    new_segments.push(seg.clone());
                }
            }
            path.segments = new_segments;
        }
        true
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
                        .map(|p| p.1.is_ident(&trait_.ident))
                        == Some(true)
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
            struct #{name!("Wrapper")}<T: ?::core::marker::Sized>(T);

            // This should be `pub` to disturb "private associated type `MyTraitRanked::AssocTy` in public interface"
            // when deligating MyTrait
            pub mod #{name!("ranked_traits")} {

                // for ImplSelfTy
                #[allow(unused)]
                use super::{ super::*, #{name!("Wrapper")}};

                #(for (trait_, rank_loc, impls) in replacing_table.values()) {

                    // pub trait MyTraitRanked<'a, Ranked, T>
                    #[allow(unused)]
                    #[doc(hidden)]
                    pub trait #{name!("{}Ranked", &trait_.ident)}
                    #{trait_.generics.insert(*rank_loc, parse_quote!(#{name!("Rank")})).ty_generics()}
                    #{trait_.colon_token} #{&trait_.supertraits} {
                        #(for item in &trait_.items) { #{process_trait_item_for_ranked(item)} }
                    }

                    // impl <
                    //     'a, Rank, T, SelfTy: MyTrait<'a, T>
                    // > MyTraitRanked<'a, Rank, T> for SelfTy
                    impl #{trait_.generics.insert_last(parse_quote!(
                        #{name!("SelfTy")}: #{&trait_.ident} #{trait_.generics.ty_generics()}
                    )).insert_last(parse_quote!(#{name!("Rank")})).impl_generics()} #{name!("{}Ranked", &trait_.ident)}
                    #{trait_.generics.ty_generics().insert(*rank_loc, parse_quote![#{name!("Rank")}])} for #{name!("SelfTy")} {
                        #{emit_trait_items_delegate(
                            trait_,
                            quote!(<Self as #{&trait_.ident} #{trait_.generics.ty_generics()}>),
                            &parse_quote!(Self)
                        )}
                    }

                    #(for impl_ in impls) {

                        #(let g = remove_cyclic_bounds(&impl_.generics, &replacing_table)) {
                            // impl<'a, T> MyTraitRanked<'a, (), T> for Wrapper<ImplSelfTy>
                            #[allow(unused_variables)]
                            impl #{impl_.generics.impl_generics()}
                            #{name!("{}Ranked", &trait_.ident)}
                            #{g.ty_generics().insert(*rank_loc, parse_quote![()])}
                            for #{name!("Wrapper")}<#{&impl_.self_ty}> #{&g.where_clause} {
                                #{emit_impl_items_leaf(impl_)}
                            }

                            // impl<'a, T> super::super::MyTrait<'a, T> for ImplSelfTy
                            //  where Wrapper<Self>: MyTraitRanked<'a, ((((),),),), T>
                            #(for attr in &impl_.attrs) { #attr }
                            #{&impl_.defaultness} #{&impl_.unsafety} impl #{g.impl_generics()}
                            #{&trait_.ident}
                            #{&impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments}
                            for #{&impl_.self_ty} #{g.push_predicate(parse_quote!(
                                #{name!("Wrapper")}<Self>: #{name!("{}Ranked", &trait_.ident)}
                                #{impl_.generics.ty_generics().insert(*rank_loc, initial_rank.clone())}
                            )).where_clause}
                            {
                                #{emit_impl_items_delegate(impl_, quote!(
                                    <Self as #{name!("{}Ranked", &trait_.ident)}
                                    #{impl_.generics.ty_generics().insert(*rank_loc, initial_rank.clone())} >
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

                    #(let g = wrap_cyclic_bounds(&impl_.generics, &replacing_table)) {
                        // impl<'a, Rank, T> ranked_traits::MyTraitRanked<'a, (Rank,), T> for
                        // Wrapper<ImplSelfTy>
                        // where
                        //     Self: ranked_traits::MyTraitRanked<'a, Rank, T>
                        impl #{g.insert_last(parse_quote!(#{name!("Rank")})).impl_generics()}
                        #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                        #{impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments.insert(*rank_loc, parse_quote!((#{name!("Rank")},)))}
                        for #{name!("Wrapper")}<#{impl_.self_ty.as_ref()}> #{g.push_predicate(parse_quote!(
                            Self: #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)} #{&impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments.insert(*rank_loc, parse_quote!(#{name!("Rank")}))}
                        )).where_clause} {
                            #{emit_impl_main(trait_, impl_, &impl_.self_ty)}
                        }
                    }
                }
            }
        }
    }
}
