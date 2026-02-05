use proc_macro2::{Span, TokenStream};
use proc_macro_error::*;
use std::collections::HashMap;
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

/// Inserts a `Type` as a `GenericArgument::Type` at the given position
/// in the last segment's arguments of `path`.
fn path_insert_type_arg(path: &mut Path, index: usize, ty: Type) {
    let last_seg = path.segments.last_mut().unwrap();
    let arg = GenericArgument::Type(ty);
    match &mut last_seg.arguments {
        PathArguments::None => {
            let mut args = Punctuated::new();
            args.insert(index, arg);
            last_seg.arguments = PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                colon2_token: None,
                lt_token: Default::default(),
                args,
                gt_token: Default::default(),
            });
        }
        PathArguments::AngleBracketed(ref mut angle_args) => {
            angle_args.args.insert(index, arg);
        }
        PathArguments::Parenthesized(_) => {}
    }
}

/// Returns `false` for trait bounds whose path is a single segment present
/// in `replacing_table`, used to filter out bounds that will be replaced.
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

        let lit: LitBool = input.parse()?;
        let support_infinite_cycle = lit.value;

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

trait GenericsScheme {
    fn insert(&self, index: usize, param: TypeParam) -> Self;
    fn impl_generics(&self) -> TokenStream;
    fn ty_generics(&self) -> TokenStream;
}

impl GenericsScheme for Generics {
    fn insert(&self, index: usize, param: TypeParam) -> Self {
        let mut generics = self.clone();
        generics.params.insert(index, GenericParam::Type(param));
        generics
    }

    fn impl_generics(&self) -> TokenStream {
        let (impl_generics, _, _) = self.split_for_impl();
        quote!(#impl_generics)
    }

    fn ty_generics(&self) -> TokenStream {
        let (_, ty_generics, _) = self.split_for_impl();
        quote!(#ty_generics)
    }
}

impl GenericsScheme for Path {
    fn insert(&self, index: usize, param: TypeParam) -> Self {
        let mut path = self.clone();
        let ty = Type::Path(TypePath {
            qself: None,
            path: parse_quote!(#param),
        });
        path_insert_type_arg(&mut path, index, ty);
        path
    }

    fn impl_generics(&self) -> TokenStream {
        quote!()
    }

    fn ty_generics(&self) -> TokenStream {
        if let Some(last_segment) = self.segments.last() {
            let args = &last_segment.arguments;
            quote!(#args)
        } else {
            quote!()
        }
    }
}

pub fn finalize(args: FinalizeArgs) -> TokenStream {
    let random_suffix = crate::get_random();
    let name =
        |s: &str| -> Ident { Ident::new(&format!("{}{}", s, &random_suffix), Span::call_site()) };

    // Mapping which maps trait path (with no args) to corresponding impl item
    let mut traits_impls: HashMap<Path, Vec<_>> = HashMap::new();

    for item_impl in args.contents {
        let mut trait_path = item_impl.trait_.clone().unwrap().1;
        if let Some(last_seg) = trait_path.segments.last_mut() {
            last_seg.arguments = PathArguments::None;
        }
        traits_impls.entry(trait_path).or_default().push(item_impl);
    }

    let replacing_table: HashMap<Ident, (usize, Path)> = args
        .traits
        .iter()
        .map(|trait_| {
            let ident = &trait_.ident;
            let g = &trait_.generics;
            let loc = g
                .params
                .iter()
                .position(|param| !matches!(param, GenericParam::Lifetime(_)))
                .unwrap_or(g.params.len());
            let ranked_ident_str = format!("{}Ranked", ident);
            let ranked_ident = name(ranked_ident_str.as_str());
            let ranked_path: Path = parse_quote!(#ranked_ident);
            (ident.clone(), (loc, ranked_path))
        })
        .collect();

    let mut output = TokenStream::new();
    for trait_ in &args.traits {
        let ident = &trait_.ident;
        let Some(impls) = traits_impls.get(&parse_quote!(#ident)) else {
            emit_warning!(ident, "trait '{}' has no implementations", ident);
            continue;
        };

        let g = &trait_.generics;
        let &(loc, ref ranked_path) = replacing_table.get(ident).unwrap();
        let initial_rank = get_initial_rank(args.recurse_level);

        let make_delegated_items =
            |ranked_bound: &Path, renamer: &mut crate::GenericRenamer| -> Vec<TokenStream> {
                trait_
                    .items
                    .iter()
                    .map(|item| match item {
                        TraitItem::Fn(method) => {
                            let mut sig = method.sig.clone();
                            let mut sig_renamer = renamer.clone();
                            for param in &sig.generics.params {
                                match param {
                                    GenericParam::Lifetime(lt) => {
                                        let lifetime = &lt.lifetime;
                                        sig_renamer
                                            .lifetime_renames
                                            .retain(|(old, _)| old != lifetime);
                                    }
                                    GenericParam::Type(tp) => {
                                        let ident = &tp.ident;
                                        sig_renamer.ident_renames.retain(|(old, _)| old != ident);
                                    }
                                    GenericParam::Const(cp) => {
                                        let ident = &cp.ident;
                                        sig_renamer.ident_renames.retain(|(old, _)| old != ident);
                                    }
                                }
                            }
                            sig_renamer.visit_signature_mut(&mut sig);
                            let method_ident = &sig.ident;
                            let call_args: Vec<TokenStream> = sig
                                .inputs
                                .iter_mut()
                                .enumerate()
                                .map(|(num, arg)| match arg {
                                    FnArg::Receiver(receiver) => {
                                        let self_token = &receiver.self_token;
                                        quote!(#self_token)
                                    }
                                    FnArg::Typed(pat_type) => {
                                        if !matches!(*pat_type.pat, Pat::Ident(_)) {
                                            *pat_type.pat = Pat::Ident(PatIdent {
                                                attrs: vec![],
                                                by_ref: None,
                                                mutability: None,
                                                ident: name(format!("param_{}_", num).as_str()),
                                                subpat: None,
                                            });
                                        }
                                        let pat = &pat_type.pat;
                                        quote!(#pat)
                                    }
                                })
                                .collect();
                            quote! {
                                #sig {
                                    <Self as #ranked_bound>::#method_ident(#(#call_args),*)
                                }
                            }
                        }
                        TraitItem::Type(assoc_type) => {
                            let type_ident = &assoc_type.ident;
                            let generics = &assoc_type.generics;
                            quote! {
                                type #type_ident #generics = <Self as #ranked_bound>::#type_ident;
                            }
                        }
                        TraitItem::Const(assoc_const) => {
                            let const_ident = &assoc_const.ident;
                            let ty = &assoc_const.ty;
                            quote! {
                                const #const_ident: #ty = <Self as #ranked_bound>::#const_ident;
                            }
                        }
                        _ => quote!(),
                    })
                    .collect()
            };

        output.extend(quote! {
            #[allow(unused)]
            #{&trait_.trait_token} #ranked_path #{g.insert(loc, parse_quote!(#{name("Rank")})).ty_generics()}
            #{trait_.colon_token} #{&trait_.supertraits} {
                #(for item in &trait_.items) { #item }
            }
        });

        for impl_ in impls {
            let impl_trait_path = impl_.trait_.as_ref().unwrap().1.clone();
            let make_ranked_path_from_impl = |rank_ty: Type| -> Path {
                let mut path: Path = parse_quote!(#ranked_path);
                if let (Some(from), Some(to)) =
                    (impl_trait_path.segments.last(), path.segments.last_mut())
                {
                    to.arguments = from.arguments.clone();
                }
                path_insert_type_arg(&mut path, loc, rank_ty);
                path
            };
            let ranked_bound = make_ranked_path_from_impl(initial_rank.clone());
            let ranked_bound_end = make_ranked_path_from_impl(parse_quote!(()));

            let mut modified_impl = impl_.clone();
            TraitReplacer(replacing_table.clone(), parse_quote!((#{name("Rank")},)))
                .visit_path_mut(&mut modified_impl.trait_.as_mut().unwrap().1);
            TraitReplacer(replacing_table.clone(), parse_quote!(#{name("Rank")}))
                .visit_item_impl_mut(&mut modified_impl);
            modified_impl
                .generics
                .params
                .push(parse_quote!(#{name("Rank")}));
            let ranked_bound_with_rank = make_ranked_path_from_impl(parse_quote!(#{name("Rank")}));
            modified_impl
                .generics
                .where_clause
                .get_or_insert(WhereClause {
                    where_token: Default::default(),
                    predicates: Default::default(),
                })
                .predicates
                .push(parse_quote!(Self: #ranked_bound_with_rank));

            if args.support_infinite_cycle {
                for (num, item) in modified_impl.items.iter_mut().enumerate() {
                    if let ImplItem::Fn(ImplItemFn { sig, block, .. }) = item {
                        let old_block = block.clone();
                        *block = parse_quote! {
                            {
                                let _ = Self::#{name("get_cell")}(#num).set( <Self as #ranked_bound>::#{&sig.ident} as _);
                                #old_block
                            }
                        };
                    }
                }
            }

            let cycle_items: Vec<TokenStream> = impl_
                .items
                .iter()
                .enumerate()
                .map(|(id, item)| match item {
                    ImplItem::Fn(method) => {
                        let mut sig = method.sig.clone();
                        // ensure that all params are ident
                        for (num, p) in sig.inputs.iter_mut().enumerate() {
                            if let FnArg::Typed(PatType { pat, .. }) = p {
                                if !matches!(pat.as_ref(), Pat::Ident(_)) {
                                    **pat = Pat::Ident(PatIdent {
                                        attrs: vec![],
                                        by_ref: None,
                                        mutability: None,
                                        ident: name(format!("param_{}_", num).as_str()),
                                        subpat: None,
                                    });
                                }
                            }
                        }
                        quote! {
                            #sig {
                                #(if args.support_infinite_cycle) {
                                    /// SAFETY:
                                    #[allow(unused_unsafe)]
                                    unsafe {
                                        ::core::mem::transmute::<
                                            _,
                                            #{&sig.unsafety} #{&sig.abi}
                                            fn(
                                                #(for p in &sig.inputs), {
                                                    #(if let FnArg::Receiver ( Receiver { ty, .. }) = p) {
                                                        #ty
                                                    }
                                                    #(if let FnArg::Typed ( PatType { ty, .. }) = p) {
                                                        #ty
                                                    }
                                                }
                                            ) #{&sig.output}
                                        >(Self::#{name("get_cell")}(#id).get().unwrap())
                                        (
                                            #(for p in &sig.inputs), {
                                                #(if let FnArg::Receiver ( Receiver { self_token, .. }) = p) {
                                                    #self_token
                                                }
                                                #(if let FnArg::Typed ( PatType { pat, .. }) = p) {
                                                    #pat
                                                }
                                            }
                                        )
                                    }
                                }
                                #(else) {
                                    ::core::unimplemented!("decycle: cycle limit reached")
                                }
                            }
                        }
                    }
                    other => quote!(#other),
                })
                .collect();

            let mut impl_generics = impl_.generics.clone();
            strip_replaced_bounds(&mut impl_generics, &replacing_table);
            output.extend(quote! {
                #modified_impl

                #[allow(unused_variables)]
                impl #{impl_generics.impl_generics()} #ranked_bound_end for #{&impl_.self_ty} #{&impl_generics.where_clause} {
                    #(#cycle_items)*
                }
            });
            let mut delegated_generics = impl_.generics.clone();
            strip_replaced_bounds(&mut delegated_generics, &replacing_table);
            let mut delegated_self_ty = (*impl_.self_ty).clone();
            let mut delegated_trait_path: Path = parse_quote!(super::#ident);
            if let Some((_, trait_path, _)) = &impl_.trait_ {
                if let (Some(from), Some(to)) = (
                    trait_path.segments.last(),
                    delegated_trait_path.segments.last_mut(),
                ) {
                    to.arguments = from.arguments.clone();
                }
            }
            let mut renamer =
                crate::randomize_impl_generics(&mut delegated_generics, random_suffix);
            renamer.visit_type_mut(&mut delegated_self_ty);
            renamer.visit_path_mut(&mut delegated_trait_path);
            let mut delegated_ranked_bound = ranked_bound.clone();
            renamer.visit_path_mut(&mut delegated_ranked_bound);
            let delegated_items = make_delegated_items(&delegated_ranked_bound, &mut renamer);
            let ranked_bound_pred: WherePredicate =
                parse_quote!(#delegated_self_ty: #delegated_ranked_bound);
            match delegated_generics.where_clause {
                Some(ref mut where_clause) => where_clause.predicates.push(ranked_bound_pred),
                None => {
                    delegated_generics.where_clause = Some(parse_quote!(where #ranked_bound_pred));
                }
            }
            output.extend(quote! {
                #(for attr in &trait_.attrs) { #attr }
                impl #{delegated_generics.impl_generics()}
                #delegated_trait_path for #delegated_self_ty #{&delegated_generics.where_clause}
                {
                    #(#delegated_items)*
                }
            });
        }
    }

    quote! {
        // this module is to prevent confliction of trait method call between ranked and non-ranked
        // traits
        #[doc(hidden)]
        mod #{name("shadowing_module")} {
            #[allow(unused)]
            use super::*;

            // Remove the non-ranked traits from namespace to prevent conflicting
            #(for ident in replacing_table.keys()) { trait #ident {} }

            #(if args.support_infinite_cycle) {
                #[allow(unused)]
                trait #{name("GetVTableKey")} {
                    extern "C" fn #{name("get_vtable_key")}(&self) {}

                    fn #{name("get_cell")}(id: ::core::primitive::usize) -> &'static ::std::sync::OnceLock<::core::primitive::usize> {
                        use ::std::sync::{Mutex, OnceLock};
                        use ::std::collections::HashMap;
                        use ::std::primitive::*;
                        static VTABLE_MAP_PARSE: OnceLock<Mutex<HashMap<(usize, usize), OnceLock<usize>>>> = OnceLock::new();
                        let map = VTABLE_MAP_PARSE.get_or_init(|| Mutex::new(HashMap::new()));
                        let mut map = map.lock().unwrap();
                        let r = map.entry((Self::#{name("get_vtable_key")} as usize, id)).or_insert(OnceLock::new());
                        // SAFETY:
                        unsafe {
                            ::core::mem::transmute(r)
                        }
                    }
                }

                impl<T: ?::core::marker::Sized> #{name("GetVTableKey")} for T {}
            }

            #output
        }
    }
}
