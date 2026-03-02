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

/// Rewrites single-segment trait paths that match the replacing table to their
/// ranked equivalents. E.g., `Evaluate` → `ranked_traits::EvaluateRanked<Rank, ()>`.
///
/// Applied twice per inductive impl:
/// 1. With `rank_type = (Rank,)` on the impl's trait path only
/// 2. With `rank_type = Rank` on the entire impl (where clause + body)
struct TraitReplacer {
    /// Maps original trait ident (e.g., "Evaluate") to (rank_loc, ranked_path)
    /// where ranked_path is a multi-segment path like `ranked_traits::EvaluateRanked`
    table: HashMap<Ident, (usize, Path)>,
    /// The rank type to insert at rank_loc
    rank_type: Type,
    /// The flag type to insert at rank_loc + 1 (after rank insertion)
    flag_type: Type,
}

impl TraitReplacer {
    /// Try to rewrite a single-segment path that matches a trait ident in the table.
    /// Returns true if a replacement was made.
    fn try_replace_path(&self, path: &mut Path) -> bool {
        if path.segments.len() == 1 {
            if let Some((rank_loc, replacement)) = self.table.get(&path.segments[0].ident) {
                let orig_args =
                    std::mem::replace(&mut path.segments[0].arguments, PathArguments::None);
                let mut new_path = replacement.clone();
                new_path.segments.last_mut().unwrap().arguments = orig_args;
                path_insert_type_arg(&mut new_path, *rank_loc, self.flag_type.clone());
                path_insert_type_arg(&mut new_path, *rank_loc, self.rank_type.clone());
                *path = new_path;
                return true;
            }
        }
        false
    }

    /// Handle paths with QSelf like `<_ as Trait>::method` or `<T as Trait>::AssocType`.
    /// The trait name appears as the first segment(s) before the QSelf position.
    fn try_replace_qself_path(&self, qself: &mut Option<QSelf>, path: &mut Path) -> bool {
        if let Some(ref mut qs) = qself {
            // In `<_ as Trait>::method`, qself.position is 1 and path is `Trait::method`.
            // Check if the first segment is a trait in our table.
            if qs.position > 0 && qs.position <= path.segments.len() {
                let first_ident = &path.segments[0].ident;
                if let Some((rank_loc, replacement)) = self.table.get(first_ident) {
                    let orig_args =
                        std::mem::replace(&mut path.segments[0].arguments, PathArguments::None);
                    // Build the replacement: replace segments[0] with the ranked path segments
                    let mut new_segments: Punctuated<PathSegment, Token![::]> = Punctuated::new();
                    for seg in &replacement.segments {
                        new_segments.push(seg.clone());
                    }
                    // Apply original type args + insert rank/flag on the last replacement segment
                    new_segments.last_mut().unwrap().arguments = orig_args;
                    {
                        let last_seg = new_segments.last_mut().unwrap();
                        let mut temp_path: Path = Path {
                            leading_colon: None,
                            segments: std::iter::once(last_seg.clone()).collect(),
                        };
                        path_insert_type_arg(&mut temp_path, *rank_loc, self.flag_type.clone());
                        path_insert_type_arg(&mut temp_path, *rank_loc, self.rank_type.clone());
                        *last_seg = temp_path.segments.into_iter().next().unwrap();
                    }
                    // Append remaining segments after the trait (e.g., `::method`)
                    for seg in path.segments.iter().skip(1) {
                        new_segments.push(seg.clone());
                    }
                    // Update QSelf position to account for the replacement having more segments
                    qs.position = qs.position - 1 + replacement.segments.len();
                    path.segments = new_segments;
                    return true;
                }
            }
        }
        false
    }
}

impl syn::visit_mut::VisitMut for TraitReplacer {
    fn visit_path_mut(&mut self, path: &mut Path) {
        if !self.try_replace_path(path) {
            syn::visit_mut::visit_path_mut(self, path);
        }
    }

    fn visit_expr_path_mut(&mut self, expr_path: &mut ExprPath) {
        if !self.try_replace_qself_path(&mut expr_path.qself, &mut expr_path.path) {
            syn::visit_mut::visit_expr_path_mut(self, expr_path);
        }
    }

    fn visit_type_path_mut(&mut self, type_path: &mut TypePath) {
        if !self.try_replace_qself_path(&mut type_path.qself, &mut type_path.path) {
            syn::visit_mut::visit_type_path_mut(self, type_path);
        }
    }
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

/// Returns true if the signature has type or const generic parameters
/// (not just lifetimes), or has `impl Trait` in argument position.
/// Such methods can't be stored as function pointers in the vtable
/// because they're not monomorphized yet.
fn sig_has_type_params(sig: &Signature) -> bool {
    sig.generics
        .params
        .iter()
        .any(|p| !matches!(p, GenericParam::Lifetime(_)))
        || sig.inputs.iter().any(|input| {
            if let FnArg::Typed(PatType { ty, .. }) = input {
                matches!(**ty, Type::ImplTrait(_))
            } else {
                false
            }
        })
}

fn emit_impl_items_leaf(impl_: &ItemImpl, support_infinite_cycle: bool) -> TokenStream {
    let mut output = TokenStream::new();

    for (ix, item) in impl_.items.iter().enumerate() {
        match item {
            ImplItem::Fn(ImplItemFn {
                defaultness, sig, ..
            }) => {
                let mut sig = sig.clone();
                // Don't replace Self — the leaf impl is for Wrapper<SelfTy>,
                // so Self should resolve to Wrapper<SelfTy> to match the ranked trait.
                // Only desugar impl Trait (pass Self as replacement = no-op for Self).
                replace_self_and_desugar_impl_trait(&mut sig, &parse_quote!(Self));

                for (param_ix, input) in sig.inputs.iter_mut().enumerate() {
                    input.reduce_pat(param_ix);
                }

                // Can't use vtable for methods with type/const params —
                // generic methods can't be coerced to a single function pointer.
                if support_infinite_cycle && !sig_has_type_params(&sig) {
                    let fn_type_params: Vec<TokenStream> = sig
                        .inputs
                        .iter()
                        .map(|p| match p {
                            FnArg::Receiver(Receiver { ty, .. }) => quote!(#ty),
                            FnArg::Typed(PatType { ty, .. }) => quote!(#ty),
                        })
                        .collect();

                    let fn_call_args: Vec<TokenStream> = sig
                        .inputs
                        .iter()
                        .map(|p| match p {
                            FnArg::Receiver(Receiver { self_token, .. }) => quote!(#self_token),
                            FnArg::Typed(PatType { pat, .. }) => quote!(#pat),
                        })
                        .collect();

                    output.extend(quote! {
                        #defaultness #sig {
                            #[allow(unused_unsafe)]
                            unsafe {
                                ::core::mem::transmute::<
                                    _,
                                    #{&sig.unsafety} #{&sig.abi}
                                    fn(
                                        #(#fn_type_params),*
                                    ) #{&sig.output}
                                >(Self::#{name!("get_cell")}(#ix).get().unwrap())
                                (
                                    #(#fn_call_args),*
                                )
                            }
                        }
                    });
                } else {
                    output.extend(quote! {
                        #defaultness #sig {
                            ::core::unimplemented!("decycle: cycle limit reached")
                        }
                    });
                }
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

                **ty = Type::Path(TypePath {
                    qself: None,
                    path: param_name.into(),
                });
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
    pub support_infinite_cycle: bool,
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

        let support_infinite_cycle = if input.is_empty() {
            false
        } else {
            let lit: LitBool = input.parse()?;
            lit.value
        };

        Ok(FinalizeArgs {
            working_list,
            traits,
            contents,
            recurse_level,
            support_infinite_cycle,
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
        let support_infinite_cycle = &self.support_infinite_cycle;

        tokens.extend(quote! {
            #crate_identity
            #crate_version
            [ #(#working_list),* ]
            { #(#traits),* }
            { #(#contents),* }
            #recurse_level
            #support_infinite_cycle
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

    // Build the TraitReplacer table: maps trait ident → (rank_loc, ranked_path)
    let trait_replacer_table: HashMap<Ident, (usize, Path)> = replacing_table
        .iter()
        .map(|(ident, (_, rank_loc, _))| {
            let ranked_path: Path = parse_quote!(
                #{name!("ranked_traits")}::#{name!("{}Ranked", ident)}
            );
            (ident.clone(), (*rank_loc, ranked_path))
        })
        .collect();

    quote! {
        // this module is to prevent confliction of trait method call between ranked and non-ranked
        // traits
        #[doc(hidden)]
        mod #{name!("shadowing_module")} {

            #(if args.support_infinite_cycle) {
                trait #{name!("GetVTableKey")} {
                    extern "C" fn #{name!("get_vtable_key")}(&self) {}

                    fn #{name!("get_cell")}(id: ::core::primitive::usize) -> &'static ::std::sync::OnceLock<::core::primitive::usize> {
                        use ::std::sync::{Mutex, OnceLock};
                        use ::std::collections::HashMap;
                        use ::std::primitive::*;
                        static #{name!("VTABLE_MAP")}: OnceLock<Mutex<HashMap<(usize, usize), OnceLock<usize>>>> = OnceLock::new();
                        let map = #{name!("VTABLE_MAP")}.get_or_init(|| Mutex::new(HashMap::new()));
                        let mut map = map.lock().unwrap();
                        let r = map.entry((Self::#{name!("get_vtable_key")} as usize, id)).or_insert(OnceLock::new());
                        // SAFETY: The OnceLock lives in a leaked HashMap entry,
                        // so the &'static lifetime is valid.
                        unsafe {
                            ::core::mem::transmute(r)
                        }
                    }
                }

                impl<T: ?::core::marker::Sized> #{name!("GetVTableKey")} for T {}
            }

            // This should be `pub` to disturb "private associated type `MyTraitRanked::AssocTy` in public interface"
            // when deligating MyTrait
            pub mod #{name!("ranked_traits")} {

                // for ImplSelfTy
                #[allow(unused)]
                use super::super::*;

                #(if args.support_infinite_cycle) {
                    #[allow(unused)]
                    use super::#{name!("GetVTableKey")};
                }

                #(for (trait_, rank_loc, impls) in replacing_table.values()) {

                    // pub trait MyTraitRanked<'a, Ranked, Flag, T>
                    #[allow(unused)]
                    #[doc(hidden)]
                    pub trait #{name!("{}Ranked", &trait_.ident)}
                    #{trait_.generics.insert(*rank_loc, parse_quote!(#{name!("Flag")})).insert(*rank_loc, parse_quote!(#{name!("Rank")})).ty_generics()}
                    #{trait_.colon_token} #{&trait_.supertraits} {
                        #(for item in &trait_.items) { #{process_trait_item_for_ranked(item)} }
                    }

                    // Base case: impl <
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
                            // Leaf: impl<'a, T> MyTraitRanked<'a, (), (), T> for ImplSelfTy
                            #[allow(unused_variables)]
                            impl #{impl_.generics.impl_generics()}
                            #{name!("{}Ranked", &trait_.ident)}
                            #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, parse_quote![()]).insert(*rank_loc, parse_quote![()])}
                            for #{&impl_.self_ty} #{&g.where_clause} {
                                #{emit_impl_items_leaf(impl_, args.support_infinite_cycle)}
                            }

                            // Final: impl<'a, T> MyTrait<'a, T> for ImplSelfTy
                            //  where Self: MyTraitRanked<'a, InitialRank, (), T>
                            #(for attr in &impl_.attrs) { #attr }
                            #{&impl_.defaultness} #{&impl_.unsafety} impl #{g.impl_generics()}
                            #{&trait_.ident}
                            #{&impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments}
                            for #{&impl_.self_ty} #{g.push_predicate(parse_quote!(
                                Self: #{name!("{}Ranked", &trait_.ident)}
                                #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, parse_quote![()]).insert(*rank_loc, initial_rank.clone())}
                            )).where_clause}
                            {
                                #{emit_impl_items_delegate(
                                    impl_,
                                    quote!(
                                        <Self as #{name!("{}Ranked", &trait_.ident)}
                                        #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, parse_quote![()]).insert(*rank_loc, initial_rank.clone())} >
                                    )
                                )}
                            }
                        }

                    }
                }
            }

            #[allow(unused)]
            use super::*;

            // Shadow original cycle-participant trait names with dummy empty traits.
            // This prevents method calls in the inductive step body from resolving
            // to the Final impls (which would reset rank to InitialRank).
            // Local definitions shadow glob imports in Rust.
            #(for trait_ in replacing_table.keys()) {
                #[allow(non_camel_case_types)]
                trait #trait_ {}
            }

            // Bring ranked traits into scope so method calls resolve to ranked versions
            // via where clause bounds.
            #[allow(unused)]
            use #{name!("ranked_traits")}::*;

            #(for (trait_, rank_loc, impls) in replacing_table.values()) {
                #(for impl_ in impls) {

                    #[allow(unused)]
                    use super::super::*;

                    // Inductive step: clone user's impl with trait paths rewritten
                    // to ranked versions via TraitReplacer.
                    // Trait path gets rank=(Rank,), body/where-clause gets rank=Rank.
                    #{
                        use syn::visit_mut::VisitMut;

                        let mut modified_impl = impl_.clone();

                        // Desugar `impl Trait` in method signatures to match the ranked
                        // trait definition (which also desugars via process_trait_item_for_ranked).
                        // This must happen BEFORE TraitReplacer so bounds inside `impl Trait`
                        // get rewritten too.
                        for item in &mut modified_impl.items {
                            if let ImplItem::Fn(ImplItemFn { sig, .. }) = item {
                                replace_self_and_desugar_impl_trait(sig, &parse_quote!(Self));
                            }
                        }

                        // Step 1: Rewrite the impl's trait path with rank=(Rank,)
                        TraitReplacer {
                            table: trait_replacer_table.clone(),
                            rank_type: parse_quote!((#{name!("Rank")},)),
                            flag_type: parse_quote!(()),
                        }.visit_path_mut(&mut modified_impl.trait_.as_mut().unwrap().1);

                        // Step 2: Rewrite all trait paths in body + where clause with rank=Rank
                        TraitReplacer {
                            table: trait_replacer_table.clone(),
                            rank_type: parse_quote!(#{name!("Rank")}),
                            flag_type: parse_quote!(()),
                        }.visit_item_impl_mut(&mut modified_impl);

                        // Add Rank as a generic parameter
                        modified_impl.generics.params.push(parse_quote!(#{name!("Rank")}));
                        if modified_impl.generics.lt_token.is_none() {
                            modified_impl.generics.lt_token = Some(Default::default());
                            modified_impl.generics.gt_token = Some(Default::default());
                        }

                        // Add Self: TraitRanked<Rank, ()> bound
                        let self_ranked_bound: WherePredicate = parse_quote!(
                            Self: #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                            #{impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments
                                .insert(*rank_loc, parse_quote![()]).insert(*rank_loc, parse_quote!(#{name!("Rank")}))}
                        );
                        modified_impl.generics
                            .where_clause
                            .get_or_insert(WhereClause {
                                where_token: Default::default(),
                                predicates: Default::default(),
                            })
                            .predicates
                            .push(self_ranked_bound);

                        // If support_infinite_cycle, prepend vtable registration to each method body
                        if args.support_infinite_cycle {
                            let ranked_bound_for_reg: Path = parse_quote!(
                                #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                                #{impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments
                                    .insert(*rank_loc, parse_quote![()])
                                    .insert(*rank_loc, parse_quote!(#{name!("Rank")}))}
                            );
                            for (num, item) in modified_impl.items.iter_mut().enumerate() {
                                if let ImplItem::Fn(ImplItemFn { sig, block, .. }) = item {
                                    if !sig_has_type_params(sig) {
                                        let old_block = block.clone();
                                        let method_ident = &sig.ident;
                                        *block = parse_quote! {
                                            {
                                                let _ = Self::#{name!("get_cell")}(#num).set(
                                                    <Self as #ranked_bound_for_reg>::#method_ident as _
                                                );
                                                #old_block
                                            }
                                        };
                                    }
                                }
                            }
                        }

                        quote!(
                            #[allow(unused_variables, unused_unsafe)]
                            #modified_impl
                        )
                    }
                }
            }
        }
    }
}
