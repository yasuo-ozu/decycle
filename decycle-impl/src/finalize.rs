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
}

impl TraitReplacer {
    /// Try to rewrite a single-segment path, or a two-segment `Trait::method` value/type
    /// path, that names a trait ident in the table — either form optionally prefixed with
    /// a no-op leading `self::` (`self::Trait`, `self::Trait::method`). Returns true if a
    /// replacement was made; `path` is left untouched on a non-match (the self:: peel is
    /// only used to LOOK UP the match, never applied unless one is found).
    fn try_replace_path(&self, path: &mut Path) -> bool {
        let offset = self_offset(path);
        let rest_len = path.segments.len() - offset;
        if rest_len != 1 && rest_len != 2 {
            return false;
        }
        let Some((rank_loc, replacement)) = self.table.get(&path.segments[offset].ident) else {
            return false;
        };
        let orig_args =
            std::mem::replace(&mut path.segments[offset].arguments, PathArguments::None);
        let mut new_path = replacement.clone();
        new_path.segments.last_mut().unwrap().arguments = orig_args;
        path_insert_type_arg(&mut new_path, *rank_loc, self.rank_type.clone());
        if rest_len == 2 {
            // `Trait::method` -> `ranked_traits::TraitRanked<Rank, ...>::method`
            new_path.segments.push(path.segments[offset + 1].clone());
        }
        *path = new_path;
        true
    }

    /// Handle paths with QSelf like `<_ as Trait>::method` or `<T as Trait>::AssocType`
    /// (also `<_ as self::Trait>::method`). The trait name appears as the first segment
    /// before the QSelf position (optionally preceded by a no-op leading `self`).
    fn try_replace_qself_path(&self, qself: &mut Option<QSelf>, path: &mut Path) -> bool {
        if let Some(ref mut qs) = qself {
            // A leading `self` can only be part of the "as Trait" portion when there's a
            // segment after it still within that portion (qs.position > 1).
            let offset = if qs.position > 1 { self_offset(path) } else { 0 };
            // In `<_ as Trait>::method`, qself.position is 1 and path is `Trait::method`.
            // Check if the first (post-`self::`) segment is a trait in our table.
            if qs.position > offset && qs.position <= path.segments.len() {
                let first_ident = &path.segments[offset].ident;
                if let Some((rank_loc, replacement)) = self.table.get(first_ident) {
                    let orig_args =
                        std::mem::replace(&mut path.segments[offset].arguments, PathArguments::None);
                    // Build the replacement: replace segments[offset] with the ranked path segments
                    let mut new_segments: Punctuated<PathSegment, Token![::]> = Punctuated::new();
                    for seg in &replacement.segments {
                        new_segments.push(seg.clone());
                    }
                    // Apply original type args + insert rank on the last replacement segment
                    new_segments.last_mut().unwrap().arguments = orig_args;
                    {
                        let last_seg = new_segments.last_mut().unwrap();
                        let mut temp_path: Path = Path {
                            leading_colon: None,
                            segments: std::iter::once(last_seg.clone()).collect(),
                        };
                        path_insert_type_arg(&mut temp_path, *rank_loc, self.rank_type.clone());
                        *last_seg = temp_path.segments.into_iter().next().unwrap();
                    }
                    // Append remaining segments after the trait (e.g., `::method`)
                    for seg in path.segments.iter().skip(offset + 1) {
                        new_segments.push(seg.clone());
                    }
                    // Update QSelf position to account for the stripped `self::` (if any)
                    // and the replacement having more segments.
                    qs.position = qs.position - offset - 1 + replacement.segments.len();
                    path.segments = new_segments;
                    return true;
                }
            }
        }
        false
    }
}

/// `1` when `path` starts with a no-op, argument-less `self` segment followed by at least
/// one more segment (`self::Trait`, `self::Trait::method`), else `0`.
fn self_offset(path: &Path) -> usize {
    if path.leading_colon.is_none()
        && path.segments.len() > 1
        && path.segments[0].ident == "self"
        && matches!(path.segments[0].arguments, PathArguments::None)
    {
        1
    } else {
        0
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
        // A bound is the cyclic one being stripped iff its LAST segment actually names a
        // #[decycle] trait — an unrelated bound (multi-segment `::std::fmt::Debug`, or any
        // single-segment trait not in the table) must survive untouched. Matching on the
        // last segment (not requiring a single segment) is deliberate: a side-bound on a
        // non-cyclic type is allowed to reference a #[decycle] trait through a qualified
        // path to reach the ORIGINAL trait (`Foreign: super::MyTrait`, bypassing ranking
        // on purpose for a type that isn't part of the cycle) — such a bound is
        // positionally fragile once copied into the generated impls at different module
        // depths (its `super::`/`crate::` prefix no longer points at the same place), so
        // it's stripped here exactly like a same-named bare reference would be.
        let is_cyclic_bound = trait_path
            .segments
            .last()
            .is_some_and(|seg| replacing_table.contains_key(&seg.ident));
        (!is_cyclic_bound).then_some((ty, trait_path))
    });
    g
}

/// Replaces every bare `Self` type (not `Self::Assoc` — no qualifying trait path is known at
/// this generics-only call site, and no existing caller needs it) with `self_ty` throughout a
/// `Generics`' param bounds and where-clause. Used when threading a preserved, `Self`-
/// mentioning side bound (e.g. `where Self: ::core::fmt::Debug`, kept verbatim by
/// `remove_cyclic_bounds` — it isn't the cyclic bound being stripped) onto a generated item
/// that has no `Self` of its own (a free fn): inside the ORIGINAL impl the bound just means
/// "this impl's own self type", so substituting it in is semantics-preserving, unlike leaving
/// the literal `Self` keyword (E0411 — "cannot find type `Self` in this scope").
fn subst_bare_self_in_generics(generics: &Generics, self_ty: &Type) -> Generics {
    struct BareSelfSubst<'a> {
        self_ty: &'a Type,
    }
    impl syn::visit_mut::VisitMut for BareSelfSubst<'_> {
        fn visit_type_mut(&mut self, ty: &mut Type) {
            if let Type::Path(TypePath { qself: None, path }) = ty {
                if path.is_ident("Self") {
                    *ty = self.self_ty.clone();
                    return;
                }
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
    let mut g = generics.clone();
    syn::visit_mut::VisitMut::visit_generics_mut(&mut BareSelfSubst { self_ty }, &mut g);
    g
}

fn emit_impl_items_leaf(
    impl_: &ItemImpl,
    trait_: &ItemTrait,
    support_infinite_cycle: bool,
    decycle: &Path,
) -> TokenStream {
    let mut output = TokenStream::new();
    let trait_ident = &trait_.ident;
    let self_targs = nonlifetime_path_args(
        &impl_
            .trait_
            .as_ref()
            .unwrap()
            .1
            .segments
            .last()
            .unwrap()
            .arguments,
    );

    for item in impl_.items.iter() {
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

                if support_infinite_cycle {
                    // Call-argument position: `variable()` emits only the bare ident for a
                    // `Pat::Ident` (dropping `mut`/`by_ref`/subpatterns, which aren't valid
                    // expression syntax), matching the delegate path's `FnArgScheme::variable`
                    // usage. A raw `quote!(#pat)` here would reproduce `mut`-qualified idents
                    // as `f(mut n)` — invalid expression syntax.
                    let fn_call_args: Vec<TokenStream> =
                        sig.inputs.iter().map(|p| p.variable()).collect();
                    let margs = type_const_idents(&sig.generics);
                    let mk = name!("__Mk_{}_{}", trait_ident, &sig.ident);
                    let fa = name!("__Fp_{}_{}", trait_ident, &sig.ident);

                    // The alias declares only the params its fn-pointer type uses (E0091);
                    // recompute the same mask here (positional, so impl-side renames of
                    // trait/method generics don't matter). Marker keys stay unfiltered.
                    let (s_used, tmask, mmask) = {
                        let tf = trait_
                            .items
                            .iter()
                            .find_map(|it| match it {
                                TraitItem::Fn(tf) if tf.sig.ident == sig.ident => Some(tf),
                                _ => None,
                            })
                            .unwrap_or_else(|| {
                                abort!(
                                    &sig.ident,
                                    "method `{}` not found on #[decycle] trait `{}`",
                                    &sig.ident,
                                    trait_ident
                                )
                            });
                        let s_ident = name!("DclSelf");
                        let s_ty: Type = parse_quote!(#s_ident);
                        let trait_args = trait_.generics.ty_generics();
                        let trait_path_full: Path =
                            parse_quote!(super::super::#trait_ident #trait_args);
                        let norm = normalize_reentry_sig(tf, &s_ty, &trait_path_full);
                        reentry_used_mask(trait_, &norm, &s_ident)
                    };
                    let mut inst: Vec<TokenStream> = Vec::new();
                    if s_used {
                        inst.push(quote!(Self));
                    }
                    inst.extend(
                        self_targs
                            .iter()
                            .zip(tmask.iter())
                            .filter(|(_, k)| **k)
                            .map(|(a, _)| quote!(#a)),
                    );
                    inst.extend(
                        margs
                            .iter()
                            .zip(mmask.iter())
                            .filter(|(_, k)| **k)
                            .map(|(i, _)| quote!(#i)),
                    );

                    let fp = fingerprint_expr(
                        decycle,
                        &quote!(Self),
                        is_syntactically_unsized(&impl_.self_ty),
                        &trait_.generics,
                        &self_targs,
                        Some(&sig.generics),
                    );

                    // The transmute names BOTH types: `usize` source (the registry hands the
                    // fn pointer out by value) and the declared alias target — never `_`.
                    output.extend(quote! {
                        #defaultness #sig {
                            let __dcl_f = unsafe {
                                ::core::mem::transmute::<
                                    ::core::primitive::usize,
                                    #fa #(if !inst.is_empty()) { <#(#inst),*> }
                                >(#decycle::__reentry::lookup::<
                                    #mk<Self #(for a in &self_targs) {, #a} #(for i in &margs) {, #i}>
                                >(#fp))
                            };
                            // A bare `{ __dcl_f(...) }` nested directly as this fn body's
                            // tail statement (no `unsafe` to scope) would just be a redundant
                            // brace pair (`unused_braces` in a plain, non-macro-generated
                            // shape); emit the call unwrapped in that case instead of
                            // conditionally prefixing an otherwise-pointless block.
                            #(if sig.unsafety.is_some()) {
                                unsafe { __dcl_f(#(#fn_call_args),*) }
                            }
                            #(if sig.unsafety.is_none()) {
                                __dcl_f(#(#fn_call_args),*)
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
                // Turbofish the method's own type/const generics so a param appearing only
                // in a bound (phantom) stays inferable — except when the method also takes
                // `impl Trait` (E0632 forbids explicit args then; such a phantom+impl-Trait
                // combo is uncallable in plain Rust anyway).
                let margs = type_const_idents(&sig.generics);
                let do_turbofish = !margs.is_empty() && !sig_has_impl_trait_input(&sig);
                output.extend(quote! {
                    #defaultness #sig {
                        #path::#{&sig.ident}
                        #(if do_turbofish) { ::<#(#margs),*> }
                        (
                            #(for input in &sig.inputs), {
                                #{input.variable()}
                            }
                        )
                    }
                })
            }
            ImplItem::Type(ImplItemType {
                ident, generics, ..
            }) => {
                // A GAT's own params (`type Assoc<'a, T>;`) must be threaded onto the RHS
                // too — `= #path::#ident;` silently dropped them, defaulting every param
                // and very likely resolving to the wrong (or no) instantiation.
                let targs = generics.ty_generics();
                output.extend(quote! {
                    type #ident #generics = #path::#ident #targs;
                })
            }
            ImplItem::Const(ImplItemConst { ident, ty, .. }) => output.extend(quote! {
                const #ident: #ty = #path::#ident;
            }),
            other => abort!(
                other,
                "unsupported item in an impl of a #[decycle] trait"
            ),
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
    if let TraitItem::Fn(TraitItemFn { sig, .. }) = &mut item {
        // Keep the default body (if any) verbatim: a leaf/inductive impl of the ranked
        // trait that doesn't override a defaulted method must fall back to it, exactly
        // like an impl of the original trait would — stripping it here made every
        // defaulted method abstract on the ranked trait, so an impl omitting it hit
        // E0046 even though the original trait supplies a default.
        replace_self_and_desugar_impl_trait(sig, &parse_quote!(Self));
    }
    item
}

/// Replaces `Self` with a concrete type in a type tree, including `Self::Assoc`
/// projections (which become `<S as Trait<...>>::Assoc` — a bare `S::Assoc` would not
/// resolve on a generic `S`).
struct SelfSubst<'a> {
    s_ty: &'a Type,
    trait_path: &'a Path,
}

impl syn::visit_mut::VisitMut for SelfSubst<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if let Type::Path(TypePath { qself: None, path }) = ty {
            if path.is_ident("Self") {
                *ty = self.s_ty.clone();
                return;
            }
            if path.leading_colon.is_none()
                && path.segments.len() > 1
                && path.segments[0].ident == "Self"
            {
                let mut segments: Punctuated<PathSegment, Token![::]> =
                    self.trait_path.segments.clone();
                for seg in path.segments.iter().skip(1) {
                    segments.push(seg.clone());
                }
                *ty = Type::Path(TypePath {
                    qself: Some(QSelf {
                        lt_token: Default::default(),
                        ty: Box::new(self.s_ty.clone()),
                        position: self.trait_path.segments.len(),
                        as_token: Some(Default::default()),
                        gt_token: Default::default(),
                    }),
                    path: Path {
                        leading_colon: self.trait_path.leading_colon,
                        segments,
                    },
                });
                return;
            }
        }
        syn::visit_mut::visit_type_mut(self, ty);
    }
}

/// Gives every elided lifetime (`&T`, `'_`) a fresh name. Fn-pointer types and free fns
/// have no `self` elision rule, so the re-entry fn / floor alias signatures must be fully
/// named before output elision can be resolved against the receiver.
struct LtNamer {
    fresh: Vec<Lifetime>,
    counter: usize,
}

impl LtNamer {
    fn fresh_lt(&mut self) -> Lifetime {
        let lt = Lifetime::new(&format!("'__dcl{}", self.counter), Span::call_site());
        self.counter += 1;
        self.fresh.push(lt.clone());
        lt
    }
}

impl syn::visit_mut::VisitMut for LtNamer {
    fn visit_type_reference_mut(&mut self, tr: &mut TypeReference) {
        match &tr.lifetime {
            None => tr.lifetime = Some(self.fresh_lt()),
            Some(l) if l.ident == "_" => tr.lifetime = Some(self.fresh_lt()),
            _ => {}
        }
        self.visit_type_mut(&mut tr.elem);
    }

    fn visit_lifetime_mut(&mut self, l: &mut Lifetime) {
        if l.ident == "_" {
            *l = self.fresh_lt();
        }
    }
}

fn count_elided_lifetimes(ty: &Type) -> usize {
    struct Counter(usize);
    impl<'ast> syn::visit::Visit<'ast> for Counter {
        fn visit_type_reference(&mut self, tr: &'ast TypeReference) {
            match &tr.lifetime {
                None => self.0 += 1,
                Some(l) if l.ident == "_" => self.0 += 1,
                _ => {}
            }
            self.visit_type(&tr.elem);
        }
        fn visit_lifetime(&mut self, l: &'ast Lifetime) {
            if l.ident == "_" {
                self.0 += 1;
            }
        }
    }
    let mut c = Counter(0);
    syn::visit::Visit::visit_type(&mut c, ty);
    c.0
}

fn subst_elided_lifetimes(ty: &mut Type, subst: &Lifetime) {
    struct Subst(Lifetime);
    impl syn::visit_mut::VisitMut for Subst {
        fn visit_type_reference_mut(&mut self, tr: &mut TypeReference) {
            match &tr.lifetime {
                None => tr.lifetime = Some(self.0.clone()),
                Some(l) if l.ident == "_" => tr.lifetime = Some(self.0.clone()),
                _ => {}
            }
            self.visit_type_mut(&mut tr.elem);
        }
        fn visit_lifetime_mut(&mut self, l: &mut Lifetime) {
            if l.ident == "_" {
                *l = self.0.clone();
            }
        }
    }
    syn::visit_mut::VisitMut::visit_type_mut(&mut Subst(subst.clone()), ty);
}

fn distinct_lifetimes_in(tys: impl Iterator<Item = Type>) -> Vec<Lifetime> {
    struct Collector(Vec<Lifetime>);
    impl<'ast> syn::visit::Visit<'ast> for Collector {
        fn visit_lifetime(&mut self, l: &'ast Lifetime) {
            if l.ident != "_" && l.ident != "static" && !self.0.iter().any(|s| s.ident == l.ident)
            {
                self.0.push(l.clone());
            }
        }
    }
    let mut c = Collector(Vec::new());
    for ty in tys {
        syn::visit::Visit::visit_type(&mut c, &ty);
    }
    c.0
}

/// The type/const generic args of a path's last segment (lifetimes and associated-type
/// constraints dropped) — the marker/alias instantiation list for a trait path.
fn nonlifetime_path_args(args: &PathArguments) -> Vec<GenericArgument> {
    match args {
        PathArguments::AngleBracketed(ab) => ab
            .args
            .iter()
            .filter(|a| matches!(a, GenericArgument::Type(_) | GenericArgument::Const(_)))
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

fn type_const_idents(generics: &Generics) -> Vec<Ident> {
    generics
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(t) => Some(t.ident.clone()),
            GenericParam::Const(c) => Some(c.ident.clone()),
            _ => None,
        })
        .collect()
}

fn sig_has_impl_trait_input(sig: &Signature) -> bool {
    sig.inputs.iter().any(
        |i| matches!(i, FnArg::Typed(PatType { ty, .. }) if matches!(&**ty, Type::ImplTrait(_))),
    )
}

/// A method whose floor key depends on an instantiation rule 2 cannot know.
fn method_is_generic(sig: &Signature) -> bool {
    sig.generics
        .params
        .iter()
        .any(|p| !matches!(p, GenericParam::Lifetime(_)))
        || sig_has_impl_trait_input(sig)
}

/// Bound-free declaration form for marker/alias generics (`T`, `const N: usize`, `'a`).
///
/// A type param is declared `?Sized` (F-M2): absent any bound, a generic item's own type
/// params default to an implicit `Sized` requirement, but the marker only ever holds one in
/// `PhantomData<*const T>` (tolerates `?Sized`) and the alias only ever uses one behind a
/// reference (`&V`, also `?Sized`-tolerant) — so a cycle whose method has a `V: ?Sized`
/// parameter (or whose target is itself unsized — F-M1) would otherwise fail E0277 at the
/// marker/alias declaration merely from being named, independent of any *use* of the param.
fn generic_param_plain(p: &GenericParam) -> TokenStream {
    match p {
        GenericParam::Type(t) => {
            let i = &t.ident;
            quote!(#i: ?::core::marker::Sized)
        }
        GenericParam::Const(c) => {
            let i = &c.ident;
            let t = &c.ty;
            quote!(const #i: #t)
        }
        GenericParam::Lifetime(l) => {
            let l = &l.lifetime;
            quote!(#l)
        }
    }
}

/// Full declaration (bounds kept, defaults stripped) for re-entry fn generics.
fn generic_param_bounded(p: &GenericParam) -> TokenStream {
    match p {
        GenericParam::Type(t) => {
            let mut t = t.clone();
            t.default = None;
            t.eq_token = None;
            quote!(#t)
        }
        GenericParam::Const(c) => {
            let mut c = c.clone();
            c.default = None;
            c.eq_token = None;
            quote!(#c)
        }
        GenericParam::Lifetime(l) => quote!(#l),
    }
}

/// The impl's cyclic `where`-predicates: `(bounded type, trait ident, non-lifetime trait
/// args)` for every single-segment decycle-trait bound whose target is not a bare type
/// parameter of the impl (a bare-param target's *original*-trait obligation is not provable
/// inside the rank-rewritten impl, so such bounds are skipped — their non-generic-method
/// coverage then relies on rule 1 of the callee's own frames).
fn cyclic_where_bounds(
    impl_: &ItemImpl,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> Vec<(Type, Ident, Vec<GenericArgument>)> {
    let param_idents: std::collections::HashSet<Ident> = impl_
        .generics
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(t) => Some(t.ident.clone()),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    let Some(wc) = &impl_.generics.where_clause else {
        return out;
    };
    for pred in &wc.predicates {
        let WherePredicate::Type(pt) = pred else {
            continue;
        };
        if let Type::Path(TypePath { qself: None, path }) = &pt.bounded_ty {
            if path.segments.len() == 1 && param_idents.contains(&path.segments[0].ident) {
                continue;
            }
        }
        for b in &pt.bounds {
            let TypeParamBound::Trait(tb) = b else {
                continue;
            };
            if tb.path.segments.len() == 1 {
                let seg = &tb.path.segments[0];
                if replacing_table.contains_key(&seg.ident) {
                    out.push((
                        pt.bounded_ty.clone(),
                        seg.ident.clone(),
                        nonlifetime_path_args(&seg.arguments),
                    ));
                }
            }
        }
    }
    out
}

fn type_param_is_maybe_unsized(tp: &TypeParam, where_clause: Option<&WhereClause>) -> bool {
    let is_maybe = |bounds: &Punctuated<TypeParamBound, Token![+]>| {
        bounds.iter().any(|b| {
            matches!(
                b,
                TypeParamBound::Trait(TraitBound {
                    modifier: TraitBoundModifier::Maybe(_),
                    ..
                })
            )
        })
    };
    is_maybe(&tp.bounds)
        || where_clause.is_some_and(|wc| {
            wc.predicates.iter().any(|pred| match pred {
                WherePredicate::Type(pt) => {
                    matches!(&pt.bounded_ty, Type::Path(TypePath { qself: None, path })
                        if path.is_ident(&tp.ident))
                        && is_maybe(&pt.bounds)
                }
                _ => false,
            })
        })
}

/// A const-generic value is folded into the fingerprint only when its declared type is a
/// primitive castable `as u64` (every stable const-param type; wider values truncate,
/// signed ones sign-extend — deterministic either way). Anything else is skipped.
fn const_param_ty_foldable(ty: &Type) -> bool {
    const FOLDABLE: &[&str] = &[
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        "bool", "char",
    ];
    matches!(ty, Type::Path(TypePath { qself: None, path })
        if FOLDABLE.iter().any(|p| path.is_ident(p)))
}

/// True iff `ty` is syntactically unsized — a trait object, a slice, or the bare `str` path —
/// so `size_of::<ty>()`/`align_of::<ty>()` would not compile (F-M1). One shared predicate used
/// by every `fingerprint_expr` call site (the target fold is skipped for such a type): such
/// targets are never anonymous, so omitting them from the fingerprint loses no discriminating
/// power, and every emission site agreeing on the same predicate keeps registration and floor
/// keys consistent.
fn is_syntactically_unsized(ty: &Type) -> bool {
    match ty {
        Type::TraitObject(_) => true,
        Type::Slice(_) => true,
        Type::Path(TypePath { qself: None, path }) => path.is_ident("str"),
        _ => false,
    }
}

/// True iff every occurrence of `s_ident` among `tys` is the IMMEDIATE referent of a `&`/`&mut`
/// (never by value, never nested inside some other generic position like `Vec<S>` or `(S, S)`
/// — those need `S: Sized` regardless of being behind a reference themselves). Marking the
/// `#fa` alias's / `#re` fn's own `S` param `?Sized` is only sound when this holds (F-M1): an
/// unsized `Self` — `impl Ca for str` — is fine for `fn ca(&self, ...)` (receiver `&S`) but a
/// hypothetical by-value-`Self`-returning or -taking method would need `S: Sized` regardless,
/// exactly like it would on the ORIGINAL trait method.
fn s_ident_only_behind_ref<'a>(tys: impl Iterator<Item = &'a Type>, s_ident: &Ident) -> bool {
    struct Check<'a> {
        s_ident: &'a Ident,
        unsafe_found: bool,
    }
    fn is_bare_ident(ty: &Type, ident: &Ident) -> bool {
        matches!(ty, Type::Path(TypePath { qself: None, path }) if path.is_ident(ident))
    }
    impl<'a, 'ast> syn::visit::Visit<'ast> for Check<'a> {
        fn visit_type(&mut self, ty: &'ast Type) {
            if let Type::Reference(r) = ty {
                if is_bare_ident(&r.elem, self.s_ident) {
                    return; // a bare `&S`/`&mut S` — the safe shape, nothing more to check
                }
            } else if is_bare_ident(ty, self.s_ident) {
                self.unsafe_found = true;
                return;
            }
            syn::visit::visit_type(self, ty);
        }
    }
    let mut c = Check {
        s_ident,
        unsafe_found: false,
    };
    for ty in tys {
        syn::visit::Visit::visit_type(&mut c, ty);
    }
    !c.unsafe_found
}

/// True iff any of `tys` contains a `<T as Trait>::Assoc`-style projection (a `TypePath` with
/// a `QSelf`). The alias's `S: <ranked trait>` where-clause is only well-formed (only
/// references params the alias actually declares) when its own body needs it for exactly this
/// — a method with no associated-type usage at all (e.g. `fn describe(&self) -> &'static
/// str`) may leave a trait-level param like `T` entirely unmasked out of the alias's generic
/// list, so unconditionally emitting the bound (regardless of whether anything needs it) is an
/// E0412 "cannot find type" waiting to happen.
fn any_type_has_projection<'a>(tys: impl Iterator<Item = &'a Type>) -> bool {
    struct Check(bool);
    impl<'ast> syn::visit::Visit<'ast> for Check {
        fn visit_type_path(&mut self, tp: &'ast TypePath) {
            if tp.qself.is_some() {
                self.0 = true;
            }
            syn::visit::visit_type_path(self, tp);
        }
    }
    let mut c = Check(false);
    for ty in tys {
        syn::visit::Visit::visit_type(&mut c, ty);
    }
    c.0
}

/// The layout-fingerprint expression for one marker instantiation: a deterministic fold
/// over size/align of the target (`Self` / the rule-2 bound target) and every TYPE generic
/// argument, plus the value of every foldable const argument — `type_name` is non-injective
/// (two closures in one fn share a `{{closure}}` name), so the fingerprint keeps
/// different-layout instantiations on distinct registry keys. Registration sites and the
/// floor MUST fold the identical list in the identical order, which is why both emission
/// sites call this one helper. Params declared `?Sized` are skipped (`size_of` would not
/// compile); the target is folded unless `target_is_unsized` (F-M1 — e.g. `impl Ca for str`;
/// naming the re-entry fn still requires the target `Sized` at every ordinary registration
/// site, but a syntactically unsized target is exactly the case that isn't).
fn fingerprint_expr(
    decycle: &Path,
    target: &TokenStream,
    target_is_unsized: bool,
    trait_generics: &Generics,
    targs: &[GenericArgument],
    method_generics: Option<&Generics>,
) -> TokenStream {
    let mut expr = quote!(#decycle::__reentry::FP_SEED);
    let fold_layout = |acc: TokenStream, ty: TokenStream| {
        quote!(#decycle::__reentry::fp_fold(
            #acc,
            ::core::mem::size_of::<#ty>(),
            ::core::mem::align_of::<#ty>()
        ))
    };
    if !target_is_unsized {
        expr = fold_layout(expr, target.clone());
    }
    let tparams: Vec<&GenericParam> = trait_generics
        .params
        .iter()
        .filter(|p| !matches!(p, GenericParam::Lifetime(_)))
        .collect();
    for (param, arg) in tparams.iter().zip(targs.iter()) {
        match (param, arg) {
            (GenericParam::Type(tp), GenericArgument::Type(t))
                if !type_param_is_maybe_unsized(tp, trait_generics.where_clause.as_ref()) =>
            {
                expr = fold_layout(expr, quote!(#t));
            }
            (GenericParam::Const(cp), GenericArgument::Const(e))
                if const_param_ty_foldable(&cp.ty) =>
            {
                expr = quote!(#decycle::__reentry::fp_fold_word(#expr, (#e) as u64));
            }
            _ => {}
        }
    }
    for p in method_generics.iter().flat_map(|g| g.params.iter()) {
        match p {
            GenericParam::Type(tp)
                if !type_param_is_maybe_unsized(
                    tp,
                    method_generics.and_then(|g| g.where_clause.as_ref()),
                ) =>
            {
                let i = &tp.ident;
                expr = fold_layout(expr, quote!(#i));
            }
            GenericParam::Const(cp) if const_param_ty_foldable(&cp.ty) => {
                let i = &cp.ident;
                expr = quote!(#decycle::__reentry::fp_fold_word(#expr, (#i) as u64));
            }
            _ => {}
        }
    }
    expr
}

/// True iff the impl carries a cyclic bound whose bounded type is a bare type parameter of
/// the impl (`impl<T: Cb> …` or `where T: Cb`). Rule 1's registrations raise `Self: T`,
/// which unwinds through this impl's ranked chain into the bare param's cyclic obligations
/// at concrete ranks — but the rank-rewritten frame only has `T: CbRanked<Rank>` in scope,
/// so the registration would not compile (E0277). Rule 1 skips such impls; their floors
/// fail closed via the actionable lookup panic.
fn impl_has_bare_param_cyclic_bound(
    impl_: &ItemImpl,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> bool {
    let is_cyclic = |bounds: &Punctuated<TypeParamBound, Token![+]>| {
        bounds.iter().any(|b| match b {
            TypeParamBound::Trait(tb) => {
                tb.path.segments.len() == 1
                    && replacing_table.contains_key(&tb.path.segments[0].ident)
            }
            _ => false,
        })
    };
    let param_idents: std::collections::HashSet<&Ident> = impl_
        .generics
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(t) => Some(&t.ident),
            _ => None,
        })
        .collect();
    impl_
        .generics
        .params
        .iter()
        .any(|p| matches!(p, GenericParam::Type(tp) if is_cyclic(&tp.bounds)))
        || impl_.generics.where_clause.as_ref().is_some_and(|wc| {
            wc.predicates.iter().any(|pred| match pred {
                WherePredicate::Type(pt) => {
                    matches!(&pt.bounded_ty, Type::Path(TypePath { qself: None, path })
                        if path.segments.len() == 1
                            && param_idents.contains(&path.segments[0].ident))
                        && is_cyclic(&pt.bounds)
                }
                _ => false,
            })
        })
}

// ---------------------------------------------------------------------------------------------
// F-C1: heterogeneous side-bound cycles. Naming a re-entry fn's `S: T` obligation resolves
// through the REAL (un-ranked) impls — `impl<T: Clone> Ca for A<T> where B<T>: Cb` needs
// `B<T>: Cb`, whose only impl needs `T: Default` — so a registration is only safe to emit when
// every non-cyclic bound reachable through the impl's own cyclic-bound graph is already among
// the registering impl's own bounds. The check below is conservative and purely syntactic
// (structural type unification + string-equality predicate comparison — no semantic subtrait/
// blanket-impl reasoning) and fails closed on anything it can't establish: the caller then
// skips the registration, leaving a clean isolated lookup panic instead of an uncompilable
// macro expansion.
// ---------------------------------------------------------------------------------------------

/// Unifies `pattern` (with `pattern_vars` free) against the ground type `concrete`, returning
/// the induced substitution on success. Used to match a candidate impl's own `self_ty` pattern
/// against a cyclic bound's concrete target type. Bails (`None`) on any structural mismatch —
/// the caller fails closed on that.
fn unify_type_pattern(
    pattern_vars: &std::collections::HashSet<Ident>,
    pattern: &Type,
    concrete: &Type,
) -> Option<HashMap<Ident, Type>> {
    if let Type::Path(TypePath { qself: None, path }) = pattern {
        if path.leading_colon.is_none()
            && path.segments.len() == 1
            && path.segments[0].arguments == PathArguments::None
            && pattern_vars.contains(&path.segments[0].ident)
        {
            let mut m = HashMap::new();
            m.insert(path.segments[0].ident.clone(), concrete.clone());
            return Some(m);
        }
    }
    match (pattern, concrete) {
        (
            Type::Path(TypePath { qself: None, path: pp }),
            Type::Path(TypePath { qself: None, path: cp }),
        ) => {
            if pp.leading_colon.is_some() != cp.leading_colon.is_some()
                || pp.segments.len() != cp.segments.len()
            {
                return None;
            }
            let mut out = HashMap::new();
            for (ps, cs) in pp.segments.iter().zip(cp.segments.iter()) {
                if ps.ident != cs.ident {
                    return None;
                }
                match (&ps.arguments, &cs.arguments) {
                    (PathArguments::None, PathArguments::None) => {}
                    (PathArguments::AngleBracketed(pa), PathArguments::AngleBracketed(ca)) => {
                        if pa.args.len() != ca.args.len() {
                            return None;
                        }
                        for (pg, cg) in pa.args.iter().zip(ca.args.iter()) {
                            match (pg, cg) {
                                (GenericArgument::Type(pt), GenericArgument::Type(ct)) => {
                                    merge_subst(&mut out, unify_type_pattern(pattern_vars, pt, ct)?)?;
                                }
                                (GenericArgument::Lifetime(_), GenericArgument::Lifetime(_)) => {}
                                (GenericArgument::Const(pe), GenericArgument::Const(ce)) => {
                                    if quote!(#pe).to_string() != quote!(#ce).to_string() {
                                        return None;
                                    }
                                }
                                _ => return None,
                            }
                        }
                    }
                    _ => return None,
                }
            }
            Some(out)
        }
        (Type::Tuple(pt), Type::Tuple(ct)) => {
            if pt.elems.len() != ct.elems.len() {
                return None;
            }
            let mut out = HashMap::new();
            for (p, c) in pt.elems.iter().zip(ct.elems.iter()) {
                merge_subst(&mut out, unify_type_pattern(pattern_vars, p, c)?)?;
            }
            Some(out)
        }
        (Type::Reference(pr), Type::Reference(cr)) => {
            if pr.mutability.is_some() != cr.mutability.is_some() {
                return None;
            }
            unify_type_pattern(pattern_vars, &pr.elem, &cr.elem)
        }
        (Type::Paren(p), _) => unify_type_pattern(pattern_vars, &p.elem, concrete),
        (_, Type::Paren(c)) => unify_type_pattern(pattern_vars, pattern, &c.elem),
        _ => (quote!(#pattern).to_string() == quote!(#concrete).to_string()).then(HashMap::new),
    }
}

/// Merges `add` into `base`, failing if a variable would be bound to two syntactically
/// different types (a genuine ambiguity — the caller fails closed on that too).
fn merge_subst(base: &mut HashMap<Ident, Type>, add: HashMap<Ident, Type>) -> Option<()> {
    for (k, v) in add {
        match base.get(&k) {
            Some(existing) if quote!(#existing).to_string() != quote!(#v).to_string() => {
                return None
            }
            _ => {
                base.insert(k, v);
            }
        }
    }
    Some(())
}

/// Replaces every bare single-segment type in `subst`'s domain with its mapped type.
fn apply_type_subst(ty: &Type, subst: &HashMap<Ident, Type>) -> Type {
    struct Sub<'a>(&'a HashMap<Ident, Type>);
    impl syn::visit_mut::VisitMut for Sub<'_> {
        fn visit_type_mut(&mut self, ty: &mut Type) {
            if let Type::Path(TypePath { qself: None, path }) = ty {
                if path.leading_colon.is_none()
                    && path.segments.len() == 1
                    && path.segments[0].arguments == PathArguments::None
                {
                    if let Some(rep) = self.0.get(&path.segments[0].ident) {
                        *ty = rep.clone();
                        return;
                    }
                }
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
    let mut t = ty.clone();
    syn::visit_mut::VisitMut::visit_type_mut(&mut Sub(subst), &mut t);
    t
}

/// `generics`' own non-cyclic ("side") predicates, canonicalized as `bounded : bound` strings
/// (`remove_cyclic_bounds` already computes exactly that split) and optionally substituted —
/// used both for the registering impl's own available facts (`subst = None`) and, through a
/// reached impl's unification, for what that reached impl needs (`subst = Some(..)`).
fn side_predicate_strings(
    generics: &Generics,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
    subst: Option<&HashMap<Ident, Type>>,
) -> Vec<String> {
    let g = remove_cyclic_bounds(generics, replacing_table);
    let sub = |ty: &Type| match subst {
        Some(s) => apply_type_subst(ty, s),
        None => ty.clone(),
    };
    let mut out = Vec::new();
    for p in &g.params {
        if let GenericParam::Type(tp) = p {
            let bt = sub(&Type::Path(TypePath {
                qself: None,
                path: Path::from(tp.ident.clone()),
            }));
            for b in &tp.bounds {
                if let TypeParamBound::Trait(tb) = b {
                    out.push(format!("{} : {}", quote!(#bt), quote!(#tb)));
                }
            }
        }
    }
    if let Some(wc) = &g.where_clause {
        for pred in &wc.predicates {
            if let WherePredicate::Type(pt) = pred {
                let bt = sub(&pt.bounded_ty);
                for b in &pt.bounds {
                    if let TypeParamBound::Trait(tb) = b {
                        out.push(format!("{} : {}", quote!(#bt), quote!(#tb)));
                    }
                }
            }
        }
    }
    out
}

/// Is every non-cyclic bound reachable through `target_ty: target_trait`'s cyclic-bound graph
/// already among `registering_impl`'s own bounds? Walks the graph breadth-first: at each
/// `(trait, ty)` node, finds every impl of `trait` whose `self_ty` structurally matches `ty`
/// (`unify_type_pattern`), collects that impl's own side bounds (substituted into
/// `registering_impl`'s terms), and continues through that impl's own cyclic bounds
/// (substituted the same way). Fails closed — returns `false` — the moment anything can't be
/// established syntactically: an unknown trait, no matching impl, more than one matching impl
/// (ambiguous — treated as needing the union, but an actual mismatch between them still fails
/// via `merge_subst`/string comparison), or an impl type param left unresolved by unification.
fn reachable_side_bounds_ok(
    registering_impl: &ItemImpl,
    target_ty: &Type,
    target_trait: &Ident,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> bool {
    let own_side: std::collections::HashSet<String> =
        side_predicate_strings(&registering_impl.generics, replacing_table, None)
            .into_iter()
            .collect();

    let mut visited: std::collections::HashSet<(Ident, String)> = Default::default();
    let mut queue: std::collections::VecDeque<(Ident, Type)> = Default::default();
    queue.push_back((target_trait.clone(), target_ty.clone()));
    let mut needed: Vec<String> = Vec::new();

    while let Some((trait_ident, ty)) = queue.pop_front() {
        let key = (trait_ident.clone(), quote!(#ty).to_string());
        if !visited.insert(key) {
            continue;
        }
        let Some((_, _, impls)) = replacing_table.get(&trait_ident) else {
            return false;
        };
        let mut matched_any = false;
        for cand in impls {
            let pattern_vars: std::collections::HashSet<Ident> = cand
                .generics
                .params
                .iter()
                .filter_map(|p| match p {
                    GenericParam::Type(t) => Some(t.ident.clone()),
                    _ => None,
                })
                .collect();
            let Some(subst) = unify_type_pattern(&pattern_vars, &cand.self_ty, &ty) else {
                continue;
            };
            if pattern_vars.iter().any(|v| !subst.contains_key(v)) {
                return false;
            }
            matched_any = true;
            needed.extend(side_predicate_strings(
                &cand.generics,
                replacing_table,
                Some(&subst),
            ));
            for (sub_target, sub_trait, _) in cyclic_where_bounds(cand, replacing_table) {
                queue.push_back((sub_trait, apply_type_subst(&sub_target, &subst)));
            }
        }
        if !matched_any {
            return false;
        }
    }
    needed.iter().all(|p| own_side.contains(p))
}

/// The elision-normalized re-entry signature shape shared by the alias/re-entry emission
/// and the floor's instantiation site (both must agree on it exactly).
struct NormSig {
    /// Flattened `(pattern, type)` params; a receiver becomes a typed leading param. The
    /// pattern here is a full signature-position pattern (keeps `mut`, valid in a `fn(...)`
    /// param list) — use `arg_idents`, not this, when forwarding as call ARGUMENTS.
    params: Vec<(TokenStream, Type)>,
    /// Bare-ident forwarding tokens, parallel to `params`: `mut`/`by_ref`/subpatterns
    /// stripped, since those are signature decorations, not valid in a call-argument
    /// expression position (`f(mut n)` is a syntax error).
    arg_idents: Vec<TokenStream>,
    output_ty: Option<Type>,
    /// Fresh names given to previously-elided input lifetimes.
    fresh: Vec<Lifetime>,
    /// The desugared, pattern-reduced signature (generics carry desugared `impl Trait` params).
    sig: Signature,
}

fn normalize_reentry_sig(tf: &TraitItemFn, s_ty: &Type, trait_path_full: &Path) -> NormSig {
    let orig_sig = &tf.sig;
    let mut sig = orig_sig.clone();
    replace_self_and_desugar_impl_trait(&mut sig, s_ty);
    for (ix, input) in sig.inputs.iter_mut().enumerate() {
        input.reduce_pat(ix);
    }

    let mut params: Vec<(TokenStream, Type)> = Vec::new();
    let mut arg_idents: Vec<TokenStream> = Vec::new();
    let mut recv_ref_slot = None;
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(r) => {
                let ty = (*r.ty).clone();
                if r.reference.is_some() {
                    recv_ref_slot = Some(params.len());
                }
                params.push((quote!(__dcl_recv), ty));
                arg_idents.push(quote!(__dcl_recv));
            }
            FnArg::Typed(PatType { pat, ty, .. }) => {
                params.push((quote!(#pat), (**ty).clone()));
                let Pat::Ident(pat_ident) = pat.as_ref() else {
                    unreachable!("reduce_pat always leaves a bare Pat::Ident");
                };
                let ident = &pat_ident.ident;
                arg_idents.push(quote!(#ident));
            }
        }
    }
    let mut namer = LtNamer {
        fresh: Vec::new(),
        counter: 0,
    };
    for (_, ty) in params.iter_mut() {
        syn::visit_mut::VisitMut::visit_type_mut(
            &mut SelfSubst {
                s_ty,
                trait_path: trait_path_full,
            },
            ty,
        );
        syn::visit_mut::VisitMut::visit_type_mut(&mut namer, ty);
    }
    let mut output_ty: Option<Type> = match &sig.output {
        ReturnType::Type(_, t) => Some((**t).clone()),
        ReturnType::Default => None,
    };
    if let Some(t) = output_ty.as_mut() {
        syn::visit_mut::VisitMut::visit_type_mut(
            &mut SelfSubst {
                s_ty,
                trait_path: trait_path_full,
            },
            t,
        );
        if count_elided_lifetimes(t) > 0 {
            let subst = if let Some(ix) = recv_ref_slot {
                match &params[ix].1 {
                    Type::Reference(tr) => tr.lifetime.clone().unwrap(),
                    _ => unreachable!("reference receiver flattens to a reference type"),
                }
            } else {
                let named = distinct_lifetimes_in(params.iter().map(|(_, t)| t.clone()));
                if named.len() == 1 {
                    named.into_iter().next().unwrap()
                } else {
                    abort!(
                        orig_sig,
                        "cannot resolve the elided output lifetime for #[decycle] unbounded \
                         re-entry; name the lifetime explicitly"
                    )
                }
            };
            subst_elided_lifetimes(t, &subst);
        }
    }
    NormSig {
        params,
        arg_idents,
        output_ty,
        fresh: namer.fresh,
        sig,
    }
}

/// Which of the trait's / method's type+const params (and `S`) the normalized fn-pointer
/// type actually mentions. A type alias rejects unused generics (E0091), so the alias
/// declares — and the floor instantiates — exactly this subset, in declaration order.
fn reentry_used_mask(trait_: &ItemTrait, norm: &NormSig, s_ident: &Ident) -> (bool, Vec<bool>, Vec<bool>) {
    struct Used(std::collections::HashSet<String>);
    impl<'ast> syn::visit::Visit<'ast> for Used {
        fn visit_ident(&mut self, i: &'ast Ident) {
            self.0.insert(i.to_string());
        }
    }
    let mut used = Used(Default::default());
    for (_, ty) in &norm.params {
        syn::visit::Visit::visit_type(&mut used, ty);
    }
    if let Some(t) = &norm.output_ty {
        syn::visit::Visit::visit_type(&mut used, t);
    }
    let mask = |params: &Generics| -> Vec<bool> {
        params
            .params
            .iter()
            .filter(|p| !matches!(p, GenericParam::Lifetime(_)))
            .map(|p| match p {
                GenericParam::Type(t) => used.0.contains(&t.ident.to_string()),
                GenericParam::Const(c) => used.0.contains(&c.ident.to_string()),
                GenericParam::Lifetime(_) => unreachable!(),
            })
            .collect()
    };
    (
        used.0.contains(&s_ident.to_string()),
        mask(&trait_.generics),
        mask(&norm.sig.generics),
    )
}

fn used_lifetime_idents(norm: &NormSig) -> std::collections::HashSet<String> {
    struct Used(std::collections::HashSet<String>);
    impl<'ast> syn::visit::Visit<'ast> for Used {
        fn visit_lifetime(&mut self, l: &'ast Lifetime) {
            self.0.insert(l.ident.to_string());
        }
    }
    let mut used = Used(Default::default());
    for (_, ty) in &norm.params {
        syn::visit::Visit::visit_type(&mut used, ty);
    }
    if let Some(t) = &norm.output_ty {
        syn::visit::Visit::visit_type(&mut used, t);
    }
    used.0
}

/// Per trait × method: the `type_name` key marker ZST, the explicit fn-pointer type alias
/// (the only transmute target the floor is allowed to name), and the full-height re-entry
/// fn whose body calls the *original* trait method. Emitted inside `ranked_traits`, where
/// `super::super::T` resolves to the real trait (the dummy shadows live one module up and
/// are not in child scope).
fn emit_reentry_items(trait_: &ItemTrait, rank_loc: usize, _decycle: &Path) -> TokenStream {
    let mut out = TokenStream::new();
    let trait_ident = &trait_.ident;
    let s_ident = name!("DclSelf");
    let s_ty: Type = parse_quote!(#s_ident);
    let trait_args = trait_.generics.ty_generics();
    let trait_path_full: Path = parse_quote!(super::super::#trait_ident #trait_args);
    // The `#fa` alias's body is a bare `fn(...) -> ...` type: a `Self::Assoc` return/param
    // (normalized to `<S as Trait>::Assoc` by `SelfSubst`) is a PROJECTION, which — unlike a
    // `where`-clause requirement — must be resolved the moment the alias is NAMED with a
    // concrete `S`, i.e. at the floor (`emit_impl_items_leaf`). Resolving it through the REAL
    // trait routes through the only impl of that trait (the FINAL delegating impl), which
    // needs the FULL rank chain discharged — reintroducing exactly the static cycle the floor
    // exists to break (and, transitively, F-C1's whole-graph side-bound problem, at every
    // floor crossing rather than just an unprovable registration). The floor is always inside
    // `impl XxxRanked<rank_loc = ()> for SelfTy`, which trivially proves `S:
    // XxxRanked<..., ()>` for `S = Self` — so the alias instead projects through the RANKED
    // trait at rank `()` (a sibling item in this same `ranked_traits` module, hence no
    // `super::super::` prefix), which is exactly what that surrounding impl already provides
    // (including its own `type Assoc = ...;`, copied verbatim from the user's impl).
    let ranked_trait_ident = name!("{}Ranked", trait_ident);
    let ranked_args_at_leaf = trait_.generics.ty_generics().insert(rank_loc, parse_quote!(()));
    let ranked_trait_path_leaf: Path = parse_quote!(#ranked_trait_ident #ranked_args_at_leaf);

    let mut tg = trait_.generics.clone();
    syn::visit_mut::VisitMut::visit_generics_mut(
        &mut SelfSubst {
            s_ty: &s_ty,
            trait_path: &trait_path_full,
        },
        &mut tg,
    );
    let trait_lts: Vec<GenericParam> = tg
        .params
        .iter()
        .filter(|p| matches!(p, GenericParam::Lifetime(_)))
        .cloned()
        .collect();
    let trait_tycon: Vec<GenericParam> = tg
        .params
        .iter()
        .filter(|p| !matches!(p, GenericParam::Lifetime(_)))
        .cloned()
        .collect();
    let trait_where: Vec<WherePredicate> = tg
        .where_clause
        .as_ref()
        .map(|w| w.predicates.iter().cloned().collect())
        .unwrap_or_default();

    for item in &trait_.items {
        let TraitItem::Fn(tf) = item else { continue };
        let orig_sig = &tf.sig;
        let norm = normalize_reentry_sig(tf, &s_ty, &trait_path_full);
        // Same normalization, but any `Self::Assoc` projects through the RANKED trait at the
        // leaf instead of the real one — used ONLY for the alias's own `fn(...) -> ...` body
        // (`param_tys`/`out_tokens` below); `#re` still calls the real method, so it keeps
        // using `norm`. Masks/fresh-lifetime naming are identical either way (the projection's
        // target trait doesn't change which idents are used or how elision is resolved), so
        // everything else below still derives from `norm`.
        let norm_alias = normalize_reentry_sig(tf, &s_ty, &ranked_trait_path_leaf);
        let (s_used, tmask, mmask) = reentry_used_mask(trait_, &norm, &s_ident);
        let s_maybe_unsized = s_ident_only_behind_ref(
            norm.params
                .iter()
                .map(|(_, t)| t)
                .chain(norm.output_ty.as_ref()),
            &s_ident,
        );
        let used_lts = used_lifetime_idents(&norm);

        let m_lts: Vec<GenericParam> = norm
            .sig
            .generics
            .params
            .iter()
            .filter(|p| matches!(p, GenericParam::Lifetime(_)))
            .cloned()
            .collect();
        let m_tycon: Vec<GenericParam> = norm
            .sig
            .generics
            .params
            .iter()
            .filter(|p| !matches!(p, GenericParam::Lifetime(_)))
            .cloned()
            .collect();
        let m_where: Vec<WherePredicate> = norm
            .sig
            .generics
            .where_clause
            .as_ref()
            .map(|w| w.predicates.iter().cloned().collect())
            .unwrap_or_default();
        // Alias generics: used lifetimes, then S (if used), then used type/const params.
        let alias_lts: Vec<&GenericParam> = trait_lts
            .iter()
            .chain(m_lts.iter())
            .filter(|p| match p {
                GenericParam::Lifetime(l) => used_lts.contains(&l.lifetime.ident.to_string()),
                _ => false,
            })
            .collect();
        let alias_tycon: Vec<&GenericParam> = trait_tycon
            .iter()
            .zip(tmask.iter())
            .filter(|(_, k)| **k)
            .map(|(p, _)| p)
            .chain(
                m_tycon
                    .iter()
                    .zip(mmask.iter())
                    .filter(|(_, k)| **k)
                    .map(|(p, _)| p),
            )
            .collect();
        let fresh = &norm.fresh;
        let m_ident = &norm.sig.ident;
        let mk = name!("__Mk_{}_{}", trait_ident, m_ident);
        let fa = name!("__Fp_{}_{}", trait_ident, m_ident);
        let re = name!("__Re_{}_{}", trait_ident, m_ident);
        let unsafety = &norm.sig.unsafety;
        let abi = &norm.sig.abi;
        let phantom_ty_idents: Vec<&Ident> = trait_tycon
            .iter()
            .chain(m_tycon.iter())
            .filter_map(|p| match p {
                GenericParam::Type(t) => Some(&t.ident),
                _ => None,
            })
            .collect();
        // Call-argument forwarding: bare idents (`arg_idents`), NOT `norm.params`' full
        // signature-position patterns — those may carry `mut` (valid in the `#re` fn's own
        // param list at its declaration below, invalid as a call-argument expression).
        let param_pats: Vec<&TokenStream> = norm.arg_idents.iter().collect();
        let out_tokens = match &norm.output_ty {
            Some(t) => quote!(-> #t),
            None => quote!(),
        };
        // The alias's own body types — projecting any `Self::Assoc` through the ranked trait
        // at the leaf instead of the real one (see the comment above `ranked_trait_path_leaf`).
        let alias_param_tys: Vec<&Type> = norm_alias.params.iter().map(|(_, t)| t).collect();
        let alias_out_tokens = match &norm_alias.output_ty {
            Some(t) => quote!(-> #t),
            None => quote!(),
        };
        // Only emit the alias's `where` clause when its body actually contains an
        // associated-type projection needing it — see `any_type_has_projection`'s doc comment.
        let alias_needs_bound = any_type_has_projection(
            alias_param_tys
                .iter()
                .copied()
                .chain(norm_alias.output_ty.as_ref()),
        );
        let orig_margs = type_const_idents(&orig_sig.generics);
        let do_turbofish = !orig_margs.is_empty() && !sig_has_impl_trait_input(orig_sig);

        out.extend(quote! {
            #[allow(dead_code, non_camel_case_types)]
            #[doc(hidden)]
            pub struct #mk<#s_ident: ?::core::marker::Sized
                #(for p in &trait_tycon) {, #{generic_param_plain(p)}}
                #(for p in &m_tycon) {, #{generic_param_plain(p)}}
            >(
                ::core::marker::PhantomData<(*const #s_ident, #(for t in &phantom_ty_idents) { *const #t, })>
            );

            // A method whose signature uses `Self::Assoc` (`Self::Output` etc.) needs `S:
            // <trait>` in scope for that projection (in `alias_param_tys`/`alias_out_tokens`)
            // to resolve — via the RANKED trait at the leaf (`ranked_trait_path_leaf`), which
            // the surrounding leaf impl trivially provides for `S = Self`, not the real trait
            // (whose only impl needs the whole rank chain — see the comment above).
            #[allow(dead_code, non_camel_case_types)]
            #[doc(hidden)]
            pub type #fa<
                #(for p in &alias_lts) { #{generic_param_plain(p)}, }
                #(for l in fresh) { #l, }
                #(if s_used) { #s_ident #(if s_maybe_unsized) { : ?::core::marker::Sized } }
                #(for (ix, p) in alias_tycon.iter().enumerate()) {
                    #(if ix > 0 || s_used) {,} #{generic_param_plain(p)}
                }
            >
            #(if s_used && alias_needs_bound) {
                where
                    #s_ident: #ranked_trait_path_leaf,
            }
            = #unsafety #abi fn(#(#alias_param_tys),*) #alias_out_tokens;

            // `#unsafety #abi` must mirror the trait method: the registered fn's type has to
            // equal the `#fa` alias the floor transmutes to, or the call crosses ABIs.
            #[allow(dead_code, non_snake_case, unused, clippy::too_many_arguments)]
            #[doc(hidden)]
            pub #unsafety #abi fn #re<
                #(for p in &trait_lts) { #{generic_param_bounded(p)}, }
                #(for p in &m_lts) { #{generic_param_bounded(p)}, }
                #(for l in fresh) { #l, }
                // `?Sized` only when EVERY occurrence of `S` in this method's own signature is
                // an immediate `&`/`&mut` referent (`s_maybe_unsized`, F-M1): that's the exact
                // condition under which the ORIGINAL trait method itself would accept an
                // unsized `Self` (`impl Ca for str`'s `fn ca(&self, ...)`) — a by-value-`Self`
                // method needs `S: Sized` regardless, same as it would without this fn.
                #s_ident #(if s_maybe_unsized) { : ?::core::marker::Sized }
                #(for p in &trait_tycon) {, #{generic_param_bounded(p)}}
                #(for p in &m_tycon) {, #{generic_param_bounded(p)}}
            >(
                #(for (pat, ty) in &norm.params), { #pat: #ty }
            ) #out_tokens
            where
                #s_ident: super::super::#trait_ident #trait_args,
                #(for w in &trait_where) { #w, }
                #(for w in &m_where) { #w, }
            {
                // As in the floor (`emit_impl_items_leaf`): don't nest a redundant, always-
                // present `{ ... }` when there's no `unsafe` to scope it for.
                #(if unsafety.is_some()) {
                    unsafe {
                        <#s_ident as super::super::#trait_ident #trait_args>::#m_ident
                        #(if do_turbofish) { ::<#(#orig_margs),*> }
                        (#(#param_pats),*)
                    }
                }
                #(if unsafety.is_none()) {
                    <#s_ident as super::super::#trait_ident #trait_args>::#m_ident
                    #(if do_turbofish) { ::<#(#orig_margs),*> }
                    (#(#param_pats),*)
                }
            }
        });
    }
    out
}

/// Whether rule 1 may register `impl_`'s own `Self: T` obligation at all: not a bare-param
/// cyclic bound (`impl_has_bare_param_cyclic_bound` — its `Self: T` obligation is
/// undischargeable inside the rank-rewritten frame) and F-C1's side-bound reachability check
/// passes (naming the re-entry fn's `Self: T` obligation must be provable from `impl_`'s own
/// bounds alone).
fn rule1_registration_ok(
    trait_: &ItemTrait,
    impl_: &ItemImpl,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) -> bool {
    !impl_has_bare_param_cyclic_bound(impl_, replacing_table)
        && reachable_side_bounds_ok(impl_, &impl_.self_ty, &trait_.ident, replacing_table)
}

/// One `register::<Mk<...>>(fp, Re::<...> as usize);` statement.
fn emit_registration(
    decycle: &Path,
    trait_ident: &Ident,
    m_ident: &Ident,
    target: &TokenStream,
    targs: &[GenericArgument],
    margs: &[Ident],
    fp: TokenStream,
) -> TokenStream {
    let rt = name!("ranked_traits");
    let mk = name!("__Mk_{}_{}", trait_ident, m_ident);
    let re = name!("__Re_{}_{}", trait_ident, m_ident);
    quote! {
        #decycle::__reentry::register::<#rt::#mk<#target #(for a in targs) {, #a} #(for i in margs) {, #i}>>(
            #fp,
            #rt::#re::<#target #(for a in targs) {, #a} #(for i in margs) {, #i}> as usize
        );
    }
}

/// Rule 1's registrations for THIS method's prologue: its own instantiation plus every
/// non-generic sibling method of `trait_` (`Self: T` provable through the Final impl) — kept
/// INLINE, unconditionally, exactly as the released design (NOT hoisted into the shared
/// per-impl fn like rule 2 below — see `build_shared_registrations`'s doc comment for why:
/// duplicating the identical `Re::<Self, ...> as usize` cast at a second source location for
/// the same method hits a real, if obscure, rustc region-inference pitfall around late-bound
/// lifetimes in an under-determined fn-item-to-integer cast, `bug2.rs`'s spurious `'b: 'a`
/// demand — emitting it exactly once, inline, sidesteps it entirely).
fn build_rule1_registrations(
    trait_: &ItemTrait,
    impl_: &ItemImpl,
    current_sig: &Signature,
    rule1_ok: bool,
    decycle: &Path,
) -> TokenStream {
    if !rule1_ok {
        return TokenStream::new();
    }
    let self_targs = nonlifetime_path_args(
        &impl_
            .trait_
            .as_ref()
            .unwrap()
            .1
            .segments
            .last()
            .unwrap()
            .arguments,
    );
    let self_unsized = is_syntactically_unsized(&impl_.self_ty);
    let mut out = TokenStream::new();
    let current_margs = type_const_idents(&current_sig.generics);
    let fp = fingerprint_expr(
        decycle,
        &quote!(Self),
        self_unsized,
        &trait_.generics,
        &self_targs,
        Some(&current_sig.generics),
    );
    out.extend(emit_registration(
        decycle,
        &trait_.ident,
        &current_sig.ident,
        &quote!(Self),
        &self_targs,
        &current_margs,
        fp,
    ));
    for item in &trait_.items {
        let TraitItem::Fn(tf) = item else { continue };
        if tf.sig.ident == current_sig.ident || method_is_generic(&tf.sig) {
            continue;
        }
        let fp = fingerprint_expr(
            decycle,
            &quote!(Self),
            self_unsized,
            &trait_.generics,
            &self_targs,
            None,
        );
        out.extend(emit_registration(
            decycle,
            &trait_.ident,
            &tf.sig.ident,
            &quote!(Self),
            &self_targs,
            &[],
            fp,
        ));
    }
    out
}

/// Rule 2's cyclic-bound-sibling registrations for one impl (`X: T'` provable at the
/// cross-edge call, gated per `(target, trait)` pair by F-C1's reachability check — finer
/// grained than rule 1's whole-impl skip, since a different cyclic bound on the same impl can
/// be independently provable). Identical regardless of which method is currently descending,
/// so emitted ONCE per impl (F-M3: the released version duplicated this into every method
/// body) into a shared `#[doc(hidden)]` fn, called from every method's prologue. Unlike rule 1
/// (`build_rule1_registrations`, kept inline — see its doc comment), rule 2 targets a type
/// OTHER than `Self`, so hoisting it doesn't hit the same-method-twice cast pitfall.
fn build_shared_registrations(
    impl_: &ItemImpl,
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
    decycle: &Path,
) -> TokenStream {
    let mut out = TokenStream::new();
    for (target, trait_ident, targs) in cyclic_where_bounds(impl_, replacing_table) {
        let Some((sibling_trait, _, _)) = replacing_table.get(&trait_ident) else {
            continue;
        };
        if !reachable_side_bounds_ok(impl_, &target, &trait_ident, replacing_table) {
            continue;
        }
        let target_unsized = is_syntactically_unsized(&target);
        let target_tokens = quote!(#target);
        for item in &sibling_trait.items {
            let TraitItem::Fn(tf) = item else { continue };
            if method_is_generic(&tf.sig) {
                continue;
            }
            let fp = fingerprint_expr(
                decycle,
                &target_tokens,
                target_unsized,
                &sibling_trait.generics,
                &targs,
                None,
            );
            out.extend(emit_registration(
                decycle,
                &trait_ident,
                &tf.sig.ident,
                &target_tokens,
                &targs,
                &[],
                fp,
            ));
        }
    }
    out
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

/// One `#[decycle] use original::path::T as R;` in the consuming module: `finalize`
/// needs this to reconcile a trait definition arriving through the macro ping-pong
/// (which always carries the trait's ORIGINAL ident) with local impls/bounds that only
/// ever spell it as the local alias `R` (see `finalize`'s use of `renames`, and L-C1).
struct TraitRename {
    original: Ident,
    local: Ident,
}

impl Parse for TraitRename {
    fn parse(input: ParseStream) -> Result<Self> {
        let content;
        parenthesized!(content in input);
        let original: Ident = content.parse()?;
        content.parse::<Token![,]>()?;
        let local: Ident = content.parse()?;
        Ok(TraitRename { original, local })
    }
}

impl template_quote::ToTokens for TraitRename {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let (original, local) = (&self.original, &self.local);
        tokens.extend(quote!((#original, #local)));
    }
}

pub struct FinalizeArgs {
    pub working_list: Vec<Path>,
    pub traits: Vec<ItemTrait>,
    pub contents: Vec<ItemImpl>,
    pub recurse_level: usize,
    pub support_infinite_cycle: bool,
    /// `(original_ident, local_alias)` pairs from this module's own
    /// `#[decycle] use path::T as R;` statements (see `TraitRename`).
    pub renames: Vec<(Ident, Ident)>,
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

        let renames = if input.is_empty() {
            Vec::new()
        } else {
            let renames_content;
            bracketed!(renames_content in input);
            parse_comma_separated::<TraitRename>(&renames_content)?
                .into_iter()
                .map(|r| (r.original, r.local))
                .collect()
        };

        Ok(FinalizeArgs {
            working_list,
            traits,
            contents,
            recurse_level,
            support_infinite_cycle,
            renames,
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
        let renames: Vec<TraitRename> = self
            .renames
            .iter()
            .map(|(original, local)| TraitRename {
                original: original.clone(),
                local: local.clone(),
            })
            .collect();

        tokens.extend(quote! {
            #crate_identity
            #crate_version
            [ #(#working_list),* ]
            { #(#traits),* }
            { #(#contents),* }
            #recurse_level
            #support_infinite_cycle
            [ #(#renames),* ]
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

fn is_self_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(TypePath { qself: None, path }) if path.is_ident("Self"))
}

/// Whether `ty` contains `Self` ANYWHERE in its structure, not just as the whole type —
/// `type Assoc = Box<Self>;` / `(Self,)` are just as cyclic as a bare `type Assoc = Self;`
/// (`is_self_type` alone only catches the latter).
fn type_contains_self(ty: &Type) -> bool {
    struct FindSelf(bool);
    impl<'ast> syn::visit::Visit<'ast> for FindSelf {
        fn visit_type(&mut self, ty: &'ast Type) {
            if is_self_type(ty) {
                self.0 = true;
            }
            syn::visit::visit_type(self, ty);
        }
    }
    let mut finder = FindSelf(false);
    syn::visit::Visit::visit_type(&mut finder, ty);
    finder.0
}

/// Check for `type Assoc = Self;` in impl blocks where the trait's associated type
/// has a bound referencing a `#[decycle]` trait. This creates an infinite recursive
/// definition because the ranked trait's associated type bound refers to the original
/// trait, causing a cycle through the Final impl.
fn check_assoc_type_self(
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) {
    let decycle_idents: std::collections::HashSet<&Ident> = replacing_table.keys().collect();

    for (trait_, _, impls) in replacing_table.values() {
        for trait_item in &trait_.items {
            let TraitItem::Type(TraitItemType {
                ident: assoc_ident,
                bounds,
                ..
            }) = trait_item
            else {
                continue;
            };

            // Check if any bound on this associated type references a decycle trait.
            // Single-segment match only — a multi-segment path (`some::mod::Foo`) is a
            // DIFFERENT item that merely shares a last segment with a decycle trait.
            let has_decycle_bound = bounds.iter().any(|bound| {
                if let TypeParamBound::Trait(TraitBound { path, .. }) = bound {
                    if path.segments.len() == 1 {
                        return decycle_idents.contains(&path.segments[0].ident);
                    }
                }
                false
            });

            if !has_decycle_bound {
                continue;
            }

            // Check impls for a `Self`-containing assignment (`= Self;`, `= Box<Self>;`,
            // `= (Self,);`, …) — any of them recreate the same cycle through the Final
            // impl, not just the bare-`Self` case.
            for impl_ in impls {
                for impl_item in &impl_.items {
                    let ImplItem::Type(ImplItemType { ident, ty, .. }) = impl_item else {
                        continue;
                    };
                    if ident != assoc_ident {
                        continue;
                    }
                    if type_contains_self(ty) {
                        abort!(
                            ty,
                            "infinite recursive definition: `type {} = ...` referencing `Self` with #[decycle] trait bound",
                            assoc_ident;
                            help = assoc_ident.span() =>
                            "associated type `{}` has a bound on a #[decycle] trait, \
                             and this assignment references `Self`, creating a cycle in the ranking mechanism",
                            assoc_ident
                        );
                    }
                }
            }
        }
    }
}

/// A `#[decycle]` trait listed as a supertrait of ANOTHER `#[decycle]` trait — both in the
/// same `replacing_table` batch — makes the ranked-trait definitions mutually referential
/// in a way the rank-rewriting scheme can't discharge (E0283 at the use site: the
/// supertrait bound resolves to the shadowed dummy trait, not the ranked one). Short-term:
/// reject it outright rather than emit something that fails downstream with a confusing
/// error.
fn check_no_decycle_supertraits(
    replacing_table: &HashMap<Ident, (ItemTrait, usize, Vec<ItemImpl>)>,
) {
    let decycle_idents: std::collections::HashSet<&Ident> = replacing_table.keys().collect();

    for (trait_, _, _) in replacing_table.values() {
        for bound in &trait_.supertraits {
            let TypeParamBound::Trait(TraitBound { path, .. }) = bound else {
                continue;
            };
            if path.segments.len() == 1 && decycle_idents.contains(&path.segments[0].ident) {
                abort!(
                    path,
                    "a #[decycle] trait cannot be a supertrait of another #[decycle] trait"
                );
            }
        }
    }
}

pub fn finalize(args: FinalizeArgs) -> TokenStream {
    // Apply this module's own use-site renames (`#[decycle] use path::T as R;`) BEFORE
    // indexing traits by ident: the `ItemTrait` arriving through the macro ping-pong
    // always carries the ORIGINAL name (T), but local impls/where-bounds in THIS module
    // only ever spell it as the local alias (R, from `all_traits` in `process_module`) —
    // matching on the unrenamed ident would exclude every local impl of R, silently
    // dropping them from the output entirely (L-C1). Renaming here, before
    // `replacing_table` is built, means every downstream consumer keyed on
    // `replacing_table`'s idents (leaf/inductive impls, the re-entry engine, diagnostics)
    // sees the local name for free.
    let mut traits = args.traits.clone();
    for (original, local) in &args.renames {
        if let Some(t) = traits.iter_mut().find(|t| &t.ident == original) {
            t.ident = local.clone();
        }
    }
    let replacing_table: HashMap<Ident, (ItemTrait, usize, Vec<_>)> = traits
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

    check_assoc_type_self(&replacing_table);
    check_no_decycle_supertraits(&replacing_table);

    // The decycle crate path, recovered from the working list: `process_module` always
    // appends `#decycle::__finalize` as the final element (and the carrier-macro chain only
    // ever pops from the front), so stripping the last segment yields the caller's
    // `decycle = path` override verbatim.
    let decycle_path: Path = {
        let last: Path = args
            .working_list
            .last()
            .cloned()
            .unwrap_or_else(|| parse_quote!(::decycle::__finalize));
        let n = last.segments.len();
        Path {
            leading_colon: last.leading_colon,
            segments: last.segments.into_iter().take(n.saturating_sub(1)).collect(),
        }
    };

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

            // This should be `pub` to prevent "private associated type `MyTraitRanked::AssocTy` in public interface"
            // when delegating MyTrait
            pub mod #{name!("ranked_traits")} {

                // for ImplSelfTy
                #[allow(unused)]
                use super::super::*;

                #(for (trait_, rank_loc, impls) in replacing_table.values()) {

                    // pub trait MyTraitRanked<'a, Rank, T>
                    #[allow(unused)]
                    #[doc(hidden)]
                    pub trait #{name!("{}Ranked", &trait_.ident)}
                    #{trait_.generics.insert(*rank_loc, parse_quote!(#{name!("Rank")})).ty_generics()}
                    #{trait_.colon_token} #{&trait_.supertraits} {
                        #(for item in &trait_.items) { #{process_trait_item_for_ranked(item)} }
                    }

                    // Per trait × method: type_name key marker, explicit fn-pointer alias,
                    // and the full-height re-entry fn (unbounded mode only).
                    #(if args.support_infinite_cycle) {
                        #{emit_reentry_items(trait_, *rank_loc, &decycle_path)}
                    }

                    #(for impl_ in impls) {

                        #(let g = remove_cyclic_bounds(&impl_.generics, &replacing_table)) {
                            // Leaf: impl<'a, T> MyTraitRanked<'a, (), T> for ImplSelfTy
                            #[allow(unused_variables)]
                            impl #{impl_.generics.impl_generics()}
                            #{name!("{}Ranked", &trait_.ident)}
                            #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, parse_quote![()])}
                            for #{&impl_.self_ty} #{&g.where_clause} {
                                #{emit_impl_items_leaf(impl_, trait_, args.support_infinite_cycle, &decycle_path)}
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
                #(for (impl_ix, impl_) in impls.iter().enumerate()) {

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
                        }.visit_path_mut(&mut modified_impl.trait_.as_mut().unwrap().1);

                        // Step 2: Rewrite all trait paths in body + where clause with rank=Rank
                        TraitReplacer {
                            table: trait_replacer_table.clone(),
                            rank_type: parse_quote!(#{name!("Rank")}),
                        }.visit_item_impl_mut(&mut modified_impl);

                        // Add Rank as a generic parameter
                        modified_impl.generics.params.push(parse_quote!(#{name!("Rank")}));
                        if modified_impl.generics.lt_token.is_none() {
                            modified_impl.generics.lt_token = Some(Default::default());
                            modified_impl.generics.gt_token = Some(Default::default());
                        }

                        // Add Self: TraitRanked<Rank> bound
                        let self_ranked_bound: WherePredicate = parse_quote!(
                            Self: #{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                            #{impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments
                                .insert(*rank_loc, parse_quote!(#{name!("Rank")}))}
                        );
                        modified_impl.generics
                            .where_clause
                            .get_or_insert(WhereClause {
                                where_token: Default::default(),
                                predicates: Default::default(),
                            })
                            .predicates
                            .push(self_ranked_bound);

                        // If support_infinite_cycle, prepend the re-entry registration
                        // prologue to each method body: rule 1 (this method's own
                        // instantiation + every non-generic sibling — see
                        // `build_rule1_registrations`'s doc comment for why this stays
                        // INLINE, unlike rule 2) plus a call into a shared per-impl fn
                        // holding rule 2 (F-M3: identical across every method of this impl,
                        // so hoisted into ONE fn — emitted once — instead of duplicated into
                        // each method body). Always ahead of any call that could reach a
                        // floor. The shared fn is a free fn, not an inherent impl: it never
                        // needs `Self` (rule 2 only ever targets some OTHER cyclic-bound
                        // type), so it just takes `impl_`'s own generics verbatim — no
                        // E0207 concern (that's an impl-header-only restriction), and the
                        // cyclic where-bound is stripped (`remove_cyclic_bounds`, not
                        // `impl_.generics`): the REAL cyclic trait isn't in scope as an
                        // assumption inside the rank-rewritten caller (only its
                        // `…Ranked<Rank>` form is), so requiring it here would make every
                        // call re-hit the F-C1 obligation chain regardless of what
                        // `build_shared_registrations` decided.
                        let mut register_once_item = TokenStream::new();
                        if args.support_infinite_cycle {
                            let rule1_ok = rule1_registration_ok(trait_, impl_, &replacing_table);
                            let shared_regs =
                                build_shared_registrations(impl_, &replacing_table, &decycle_path);
                            let register_once_fn =
                                name!("__dcl_register_once_{}_{}", &trait_.ident, impl_ix);
                            // A preserved non-cyclic bound may still mention bare `Self`
                            // (`where Self: ::core::fmt::Debug`) — valid on the ORIGINAL impl,
                            // but this fn is FREE (no `Self`), so substitute the impl's own
                            // self type in before threading the where-clause onto it (else
                            // E0411 — `subst_bare_self_in_generics`'s doc comment).
                            let stripped = subst_bare_self_in_generics(
                                &remove_cyclic_bounds(&impl_.generics, &replacing_table),
                                &impl_.self_ty,
                            );
                            register_once_item = quote! {
                                #[doc(hidden)]
                                #[allow(non_snake_case, unused, dead_code)]
                                fn #register_once_fn #{stripped.impl_generics()} ()
                                #{&stripped.where_clause}
                                {
                                    #shared_regs
                                }
                            };
                            let call_targs = type_const_idents(&impl_.generics);
                            for item in modified_impl.items.iter_mut() {
                                if let ImplItem::Fn(ImplItemFn { sig, block, .. }) = item {
                                    // Splice the user's own statements into THIS block
                                    // instead of nesting their whole `Block` as a trailing
                                    // expression: `#old_block` (a full `{ ... }`) spliced
                                    // after the prologue statements would double-brace a
                                    // single-expression body (`{ 1 }` -> `{ ..prologue..; {
                                    // 1 } }`, a redundant nested block) — cosmetic, but
                                    // needlessly leaks into a downstream `cargo expand`/
                                    // `unused_braces`-sensitive setup. The prologue always
                                    // precedes with nothing left after `old_block`'s own
                                    // statements, so flattening changes nothing observable
                                    // (same order, same scope end).
                                    let old_stmts = block.stmts.clone();
                                    let rule1_regs = build_rule1_registrations(
                                        trait_,
                                        impl_,
                                        sig,
                                        rule1_ok,
                                        &decycle_path,
                                    );
                                    *block = parse_quote! {
                                        {
                                            #rule1_regs
                                            #register_once_fn
                                            #(if !call_targs.is_empty()) { ::<#(#call_targs),*> }
                                            ();
                                            #(for stmt in &old_stmts) { #stmt }
                                        }
                                    };
                                }
                            }
                        }

                        quote!(
                            #register_once_item
                            #[allow(unused_variables, unused_unsafe)]
                            #modified_impl
                        )
                    }
                }
            }
        }

        // Final impls: implement original traits by delegating to ranked traits.
        // These are outside shadowing_module so original trait names are visible.
        #(for (trait_, rank_loc, impls) in replacing_table.values()) {
            #(for impl_ in impls) {
                #(let g = remove_cyclic_bounds(&impl_.generics, &replacing_table)) {
                    #(for attr in &impl_.attrs) { #attr }
                    #{&impl_.defaultness} #{&impl_.unsafety} impl #{g.impl_generics()}
                    #{&trait_.ident}
                    #{&impl_.trait_.as_ref().unwrap().1.segments.last().unwrap().arguments}
                    for #{&impl_.self_ty} #{g.push_predicate(parse_quote!(
                        Self: #{name!("shadowing_module")}::#{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                        #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, initial_rank.clone())}
                    )).where_clause}
                    {
                        #{emit_impl_items_delegate(
                            impl_,
                            quote!(
                                <Self as #{name!("shadowing_module")}::#{name!("ranked_traits")}::#{name!("{}Ranked", &trait_.ident)}
                                #{impl_.trait_.as_ref().unwrap().1.ty_generics().insert(*rank_loc, initial_rank.clone())} >
                            )
                        )}
                    }
                }
            }
        }
    }
}
