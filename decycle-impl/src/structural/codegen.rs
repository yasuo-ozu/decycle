//! Code generation: the per-member `#[repr(transparent)]` terminator `__MTerm`, the trait impl on it
//! (body via the trait-def-inside-body pattern), and the natural type's delegating impl.

use super::*;
use crate::generics_fmt::{params_decl, params_use, wrap_angle, DeclBounds};

/// An expression that reinterprets the receiver `self` into `target`-space (a same-layout type).
/// `&self`/`&mut self` use a pointer cast; any other receiver — owned `self`, or a custom type such as
/// `self: Box<Self>` / `Pin<&mut Self>` / `Rc<Self>` — casts the whole receiver value from its declared
/// type to that type with `Self` replaced by `target` (both are same-layout because `__MTerm` is a
/// `#[repr(transparent)]` wrapper of the natural type).
fn receiver_cast(r: &syn::Receiver, target: &Type, nonce: u64) -> TokenStream {
    // Emit the receiver's OWN `self` token (not a fresh one) so its hygiene matches the method's
    // `self` parameter even when the impl arrived through a `macro_rules!` wrapper.
    let self_tok = &r.self_token;
    if r.reference.is_some() {
        if r.mutability.is_some() {
            quote! { unsafe { &mut *(#self_tok as *mut Self as *mut #target) } }
        } else {
            quote! { unsafe { &*(#self_tok as *const Self as *const #target) } }
        }
    } else {
        let src = &r.ty; // e.g. `Self` or `Box<Self>` — `Self` = this impl's Self
        let dst = subst_self(&r.ty, target);
        let cast = cast_ident(nonce);
        quote! { unsafe { #cast::<#src, #dst>(#self_tok) } }
    }
}

/// Replace every `Self` type occurrence in `ty` with `replacement`. Descends into fn-pointer
/// (`fn(&Self)`), trait-object (`dyn Fn(&Self)`) and `impl Trait` (`impl Fn(&Self)`) types too — a
/// `Self` buried in a higher-order argument must be cast into `replacement`-space like any other.
pub(crate) fn subst_self(ty: &Type, replacement: &Type) -> Type {
    match ty {
        Type::Path(tp) if tp.qself.is_none() && tp.path.is_ident("Self") => replacement.clone(),
        Type::Path(tp) => {
            let mut tp = tp.clone();
            for seg in tp.path.segments.iter_mut() {
                subst_self_in_args(&mut seg.arguments, replacement);
            }
            Type::Path(tp)
        }
        Type::Reference(r) => {
            let mut r = r.clone();
            r.elem = Box::new(subst_self(&r.elem, replacement));
            Type::Reference(r)
        }
        Type::Ptr(p) => {
            let mut p = p.clone();
            p.elem = Box::new(subst_self(&p.elem, replacement));
            Type::Ptr(p)
        }
        Type::Tuple(t) => {
            let mut t = t.clone();
            t.elems = t.elems.iter().map(|e| subst_self(e, replacement)).collect();
            Type::Tuple(t)
        }
        Type::Array(a) => {
            let mut a = a.clone();
            a.elem = Box::new(subst_self(&a.elem, replacement));
            Type::Array(a)
        }
        Type::Slice(s) => {
            let mut s = s.clone();
            s.elem = Box::new(subst_self(&s.elem, replacement));
            Type::Slice(s)
        }
        Type::Paren(p) => {
            let mut p = p.clone();
            p.elem = Box::new(subst_self(&p.elem, replacement));
            Type::Paren(p)
        }
        Type::Group(g) => {
            let mut g = g.clone();
            g.elem = Box::new(subst_self(&g.elem, replacement));
            Type::Group(g)
        }
        Type::BareFn(bf) => {
            let mut bf = bf.clone();
            for input in bf.inputs.iter_mut() {
                input.ty = subst_self(&input.ty, replacement);
            }
            if let syn::ReturnType::Type(_, t) = &mut bf.output {
                *t = Box::new(subst_self(t, replacement));
            }
            Type::BareFn(bf)
        }
        Type::TraitObject(to) => {
            let mut to = to.clone();
            subst_self_in_bounds(&mut to.bounds, replacement);
            Type::TraitObject(to)
        }
        Type::ImplTrait(it) => {
            let mut it = it.clone();
            subst_self_in_bounds(&mut it.bounds, replacement);
            Type::ImplTrait(it)
        }
        other => other.clone(),
    }
}

/// Substitute `Self` inside a path segment's arguments — both `<..>` and `Fn(..)`-sugar forms.
fn subst_self_in_args(args: &mut PathArguments, replacement: &Type) {
    match args {
        PathArguments::AngleBracketed(ab) => {
            for a in ab.args.iter_mut() {
                if let GenericArgument::Type(t) = a {
                    *t = subst_self(t, replacement);
                }
            }
        }
        PathArguments::Parenthesized(p) => {
            for t in p.inputs.iter_mut() {
                *t = subst_self(t, replacement);
            }
            if let syn::ReturnType::Type(_, t) = &mut p.output {
                *t = Box::new(subst_self(t, replacement));
            }
        }
        PathArguments::None => {}
    }
}

/// Substitute `Self` inside the trait bounds of a `dyn`/`impl Trait` type.
fn subst_self_in_bounds(
    bounds: &mut syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    replacement: &Type,
) {
    for b in bounds.iter_mut() {
        if let syn::TypeParamBound::Trait(tb) = b {
            for seg in tb.path.segments.iter_mut() {
                subst_self_in_args(&mut seg.arguments, replacement);
            }
        }
    }
}

/// A `::<T, N>` turbofish for a method's own type/const generics (lifetimes inferred), or empty.
fn method_turbofish(sig: &syn::Signature) -> TokenStream {
    let args: Vec<TokenStream> = sig
        .generics
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(t) => Some({
                let id = &t.ident;
                quote!(#id)
            }),
            GenericParam::Const(c) => Some({
                let id = &c.ident;
                quote!(#id)
            }),
            GenericParam::Lifetime(_) => None,
        })
        .collect();
    if args.is_empty() {
        quote!()
    } else {
        quote!( ::< #(#args),* > )
    }
}

pub(crate) fn codegen_scc(model: &Model, scc: &Scc) -> syn::Result<TokenStream> {
    let mut out = TokenStream::new();

    // Paired impls per (member, this-trait impl block): body on the terminator, natural delegates.
    // (The `__MTerm` structs themselves are emitted once, crate-side, by `emit_terminators` — a member
    // cyclic under several traits must not redefine its terminator per-trait.)
    for im in &model.impls {
        let m = im.self_ident.to_string();
        if !scc.contains(&m, &im.trait_key) {
            continue;
        }
        out.extend(make_impls(model, scc, im)?);
    }

    Ok(out)
}

/// `#[repr(transparent)] <vis> struct __MTerm<params>(<vis> M<params>);` — the per-member terminator, a
/// same-layout wrapper of the natural type. Its visibility (and its field's) matches `M`, so wrapping a
/// private cycle type doesn't expose it through a `pub` interface (no `private_interfaces`).
pub(crate) fn make_term_item(member: &Adt, nonce: u64) -> TokenStream {
    let term = term_ident(&member.ident.to_string(), nonce);
    let m_id = &member.ident;
    let vis = member.vis();
    let decl = params_decl(&member.generics, DeclBounds::Bare);
    let uses = params_use(&member.generics);
    let decl_angle = wrap_angle(&decl);
    let use_angle = wrap_angle(&uses);
    // Replicate the type's `#[cfg]`s so a cfg-gated cyclic type's terminator strips with it (else the
    // terminator would wrap a type rustc removed → E0412).
    let cfgs = crate::extract_cfg_attrs(member.attrs());
    quote! {
        #(#cfgs)*
        #[repr(transparent)]
        #[allow(dead_code)]
        #vis struct #term #decl_angle ( #vis #m_id #use_angle );
    }
}

fn make_impls(model: &Model, scc: &Scc, im: &ImplBlock) -> syn::Result<TokenStream> {
    let m = im.self_ident.to_string();
    let adt = model.adts.get(&m).unwrap();
    let m_ident = &adt.ident;
    let self_args = &im.self_args;
    let trait_path = im
        .item
        .trait_
        .as_ref()
        .map(|(_, p, _)| p.clone())
        .unwrap();

    // natural self type `M<self_args>` and its terminator `__MTerm<self_args>` (same layout).
    let (natural, term_ty): (Type, Type) = if self_args.is_empty() {
        let term = term_ident(&m, model.nonce);
        (parse_quote!(#m_ident), parse_quote!(#term))
    } else {
        let term = term_ident(&m, model.nonce);
        (
            parse_quote!(#m_ident< #(#self_args),* >),
            parse_quote!(#term< #(#self_args),* >),
        )
    };

    // `reduced` (all cyclic preds dropped) drives the terminator + natural impls; `local` (only wrapped
    // cyclic preds dropped) drives each method's local `__DecycleBody` impl so bare cyclic bounds still
    // pin generics for the body. `stripped_wrapped` → forwarding assertions.
    let (reduced, local, stripped_wrapped) =
        reduce_generics(&im.item.generics, &im.item.self_ty, scc, model);
    let (impl_g, _, where_g) = reduced.split_for_impl();

    // An `unsafe trait` requires `unsafe impl`: the user wrote `unsafe impl` (else E0200 on their own
    // source), so mirror that keyword onto BOTH generated impls of the trait. The local
    // `__DecycleBody` trait is safe, so its impl is unaffected.
    let unsafety = &im.item.unsafety;

    let mut assertions = TokenStream::new();
    for sw in &stripped_wrapped {
        assertions.extend(emit_forwarding_assertion(sw, scc, &reduced));
    }

    // Pass 1: associated items (types/consts). They go on BOTH the natural and terminator impls, and
    // (as decls) into each method's local `__DecycleBody` so a `Self::Assoc` in a body resolves.
    let mut term_methods = TokenStream::new();
    let mut nat_methods = TokenStream::new();
    let mut assoc_items: Vec<&ImplItem> = Vec::new();
    for it in &im.item.items {
        match it {
            ImplItem::Fn(_) => {}
            ImplItem::Type(_) | ImplItem::Const(_) => {
                assoc_items.push(it);
                nat_methods.extend(it.to_token_stream());
                term_methods.extend(it.to_token_stream());
            }
            other => nat_methods.extend(other.to_token_stream()),
        }
    }

    // Pass 2: methods.
    for it in &im.item.items {
        if let ImplItem::Fn(f) = it {
            term_methods.extend(rec_method(f, &natural, &local, &assoc_items, model.nonce)?);
            nat_methods.extend(nat_method(f, &term_ty, &trait_path, model.nonce)?);
        }
    }

    // Replicate the `#[cfg]`s of BOTH the source impl and its self type onto every generated impl:
    // if either is cfg-gated out, the generated machinery (which references the natural type) must
    // strip with it rather than reference a removed item.
    let mut cfgs = crate::extract_cfg_attrs(&im.item.attrs);
    cfgs.extend(crate::extract_cfg_attrs(adt.attrs()));

    Ok(quote! {
        #assertions
        #(#cfgs)*
        #unsafety impl #impl_g #trait_path for #term_ty #where_g {
            #term_methods
        }
        #(#cfgs)*
        #unsafety impl #impl_g #trait_path for #natural #where_g {
            #nat_methods
        }
    })
}

/// True if `ty` contains an `impl Trait` node whose bounds mention `Self` (`impl Fn(&Self)`,
/// `&impl Fn(&Self)`). Such an argument can't be cast into terminator-space — its target type
/// `impl Fn(&__Term)` is unnameable (E0562) — unlike `fn(&Self)` / `dyn Fn(&Self)`, which are.
fn has_self_mentioning_impl_trait(ty: &Type) -> bool {
    match ty {
        Type::ImplTrait(_) => mentions_self(ty),
        Type::Reference(r) => has_self_mentioning_impl_trait(&r.elem),
        Type::Ptr(p) => has_self_mentioning_impl_trait(&p.elem),
        Type::Array(a) => has_self_mentioning_impl_trait(&a.elem),
        Type::Slice(s) => has_self_mentioning_impl_trait(&s.elem),
        Type::Paren(p) => has_self_mentioning_impl_trait(&p.elem),
        Type::Group(g) => has_self_mentioning_impl_trait(&g.elem),
        Type::Tuple(t) => t.elems.iter().any(has_self_mentioning_impl_trait),
        _ => false,
    }
}

/// Body-holding method on the terminator `__MTerm`. Uses the **trait-def-inside-body** pattern: a
/// local trait `__DecycleBody` whose `__run` holds the ORIGINAL block, implemented for the
/// *natural* type. Inside that impl `Self` **is** the natural type, so the body is dropped in
/// **verbatim** — no
/// `self`/`Self` rewriting. This makes `self` inside a macro (Gap 3) and `Self`-as-constructor
/// (Gap 4) resolve correctly. The receiver is cast into natural-space at the call, and the natural
/// result is cast back up to the Rec `Self`.
fn rec_method(
    f: &syn::ImplItemFn,
    natural: &Type,
    reduced: &syn::Generics,
    assoc_items: &[&ImplItem],
    nonce: u64,
) -> syn::Result<TokenStream> {
    let attrs = &f.attrs;
    let vis = &f.vis;
    // `async fn` returns an OPAQUE future (`impl Future`). The dispatch would `transmute_copy` the
    // terminator's future into the natural type's future — two distinct anonymous types — which is
    // unsound (the layout cast reinterprets an un-polled future). The size guard catches it only when
    // the sizes differ; a same-size coincidence would be silent UB. Reject it up-front, like the
    // ranked engine rejects return-position `impl Trait`.
    if let Some(a) = &f.sig.asyncness {
        return Err(syn::Error::new_spanned(
            a,
            "#[decycle(structural)]: `async fn` is not supported — the returned future is an opaque \
             type the layout cast cannot reinterpret. Return a boxed future \
             (`-> Pin<Box<dyn Future<Output = ..>>>`) instead.",
        ));
    }
    // Return-position `impl Trait` (RPITIT) is likewise an opaque return type the layout cast cannot
    // reinterpret. Reject it up-front (the ranked engine rejects it too); an associated type is the
    // portable escape hatch since it names the return type.
    if crate::finalize::sig_has_impl_trait_output(&f.sig) {
        return Err(syn::Error::new_spanned(
            &f.sig.output,
            "#[decycle(structural)]: return-position `impl Trait` is not supported — an opaque return \
             type the layout cast cannot reinterpret. Declare an associated type and return it instead \
             (e.g. `type Output; fn m(&self) -> Self::Output`), or return a concrete/boxed type.",
        ));
    }
    // An argument-position `impl Trait` whose bound mentions `Self` (`impl Fn(&Self)`) is unnameable
    // as a cast target (`impl Fn(&__Term)` — E0562), unlike a `fn(&Self)` / `dyn Fn(&Self)` arg, which
    // ARE cast (they're concrete, same-layout as `&Self` → `&__Term`). Reject it up-front, pointing at
    // those concrete forms.
    for input in &f.sig.inputs {
        if let syn::FnArg::Typed(pt) = input {
            if has_self_mentioning_impl_trait(&pt.ty) {
                return Err(syn::Error::new_spanned(
                    &pt.ty,
                    "#[decycle(structural)]: an argument-position `impl Trait` that mentions `Self` \
                     (e.g. `impl Fn(&Self)`) is not supported — its cast target is unnameable. Use a \
                     concrete form the layout cast can reinterpret: `fn(&Self)` or `&dyn Fn(&Self)`.",
                ));
            }
        }
    }
    // Rewrite destructured / `mut` / `ref` params to fresh idents so the dispatch can forward them;
    // the original patterns are rebound at the top of the body.
    let (norm_sig, rebinds) = normalize_sig(&f.sig, nonce);
    let outer_sig = &norm_sig;
    // Splice the user body's STATEMENTS (not the whole `{ .. }` block): re-wrapping the block as
    // `{ #body }` would double-brace a single-expression body (`{ { 1 } }`), tripping `unused_braces`
    // under `#![deny(warnings)]` / `clippy -D warnings`. (The ranked engine splices statements for the
    // same reason.) The tail expression stays the tail, so the return value is unchanged.
    let body_stmts = &f.block.stmts;

    let body_tr = body_ident(nonce);
    let run = run_ident(nonce);

    // `fn __run(<normalized sig>)` — same signature, renamed.
    let mut run_sig = norm_sig.clone();
    run_sig.ident = run.clone();

    let (impl_g, _, where_g) = reduced.split_for_impl();
    let trait_g = wrap_angle(&params_decl(reduced, DeclBounds::Bare)); // `<Span, Token>` for the trait decl
    let use_g = wrap_angle(&params_use(reduced)); // `<Span, Token>` at the impl/call

    // associated-item decls (for the local trait) and defs (for the local impl), so a `Self::Assoc`
    // in the body resolves against the natural type.
    let assoc_decls: Vec<TokenStream> = assoc_items.iter().map(|it| assoc_decl(it)).collect();
    let assoc_defs: Vec<TokenStream> = assoc_items.iter().map(|it| it.to_token_stream()).collect();

    // dispatch: cast receiver + any Self-typed args into natural-space, forward the rest, call __run,
    // cast result up.
    let mut call_args: Vec<TokenStream> = Vec::new();
    if let Some(r) = outer_sig.receiver() {
        call_args.push(receiver_cast(r, natural, nonce));
    }
    call_args.extend(forward_args(outer_sig, natural, nonce)?);
    let turbofish = method_turbofish(outer_sig);
    let call = quote! {
        < #natural as #body_tr #use_g >::#run #turbofish ( #(#call_args),* )
    };
    let cast = cast_ident(nonce);
    let dispatch = match &outer_sig.output {
        ReturnType::Default => quote! { #call; },
        ReturnType::Type(..) => quote! { unsafe { #cast(#call) } },
    };

    let inline = inline_attr(attrs);
    // Replicate `#[track_caller]` / lint / hint attrs onto the body-bearing `__run` too — otherwise
    // `Location::caller()` in the body stops at the shim, and a method-level `#[allow(..)]` wouldn't
    // cover the copy holding the user's statements.
    let prop = crate::propagated_method_attrs(attrs);
    // This is the TERMINATOR (`__MTerm`) copy — an internal delegate, not the callable entry — so it
    // must NOT carry symbol attrs (`#[no_mangle]` etc.); those stay on the natural impl (`nat_method`)
    // so the requested symbol is produced exactly once.
    let outer_attrs: Vec<&Attribute> = attrs.iter().filter(|a| !crate::is_symbol_attr(a)).collect();
    Ok(quote! {
        #inline #(#outer_attrs)* #vis #outer_sig {
            trait #body_tr #trait_g : Sized {
                #(#assoc_decls)*
                #run_sig ;
            }
            impl #impl_g #body_tr #use_g for #natural #where_g {
                #(#assoc_defs)*
                #(#prop)* #run_sig { #(#rebinds)* #(#body_stmts)* }
            }
            #dispatch
        }
    })
}

/// The trait-declaration form of an associated impl item: `type Out = i64;` → `type Out;`,
/// `const N: usize = 3;` → `const N: usize;`.
fn assoc_decl(item: &ImplItem) -> TokenStream {
    match item {
        ImplItem::Type(t) => {
            let id = &t.ident;
            let (ig, _, wg) = t.generics.split_for_impl();
            quote! { type #id #ig #wg ; }
        }
        ImplItem::Const(c) => {
            let id = &c.ident;
            let ty = &c.ty;
            quote! { const #id : #ty ; }
        }
        _ => quote!(),
    }
}

/// `#[inline]` for a generated dispatch shim, unless the user's own method already carries an
/// `#[inline]` (the user attrs are cloned onto the shim, and a duplicate would trip
/// `unused_attributes`). Both shims are tiny pass-throughs; inlining lets a downstream crate collapse
/// the natural→terminator→`__run` chain to a direct call even without LTO (the user body lives in the
/// un-attributed `__run`, so this never force-duplicates user code).
fn inline_attr(attrs: &[Attribute]) -> TokenStream {
    if attrs.iter().any(|a| a.path().is_ident("inline")) {
        quote!()
    } else {
        quote!(#[inline])
    }
}

/// Delegating method on the natural type: cast the receiver (if any) into `Rec`-space, call the Rec
/// impl's trait method, and cast the result back.
fn nat_method(
    f: &syn::ImplItemFn,
    closed_rec: &Type,
    trait_path: &syn::Path,
    nonce: u64,
) -> syn::Result<TokenStream> {
    let attrs = &f.attrs;
    let vis = &f.vis;
    let (norm_sig, _) = normalize_sig(&f.sig, nonce);
    let sig = &norm_sig;
    let name = &sig.ident;
    let mut call_args: Vec<TokenStream> = Vec::new();
    if let Some(r) = sig.receiver() {
        call_args.push(receiver_cast(r, closed_rec, nonce));
    }
    call_args.extend(forward_args(sig, closed_rec, nonce)?);
    // forward the method's own type/const generics (lifetimes inferred) so non-inferable params resolve
    let turbofish = method_turbofish(sig);
    let call = quote! { < #closed_rec as #trait_path >::#name #turbofish ( #(#call_args),* ) };
    let cast = cast_ident(nonce);
    let inline = inline_attr(attrs);
    match &sig.output {
        ReturnType::Default => Ok(quote! { #inline #(#attrs)* #vis #sig { #call; } }),
        ReturnType::Type(..) => Ok(quote! {
            #inline #(#attrs)* #vis #sig { unsafe { #cast(#call) } }
        }),
    }
}

/// Forward the non-receiver parameters (receiver handled separately). An argument whose type mentions
/// `Self` (e.g. `other: &Self` in `PartialEq::eq`) is cast into `target`-space, exactly like the
/// receiver; others pass through by ident.
fn forward_args(sig: &syn::Signature, target: &Type, nonce: u64) -> syn::Result<Vec<TokenStream>> {
    let mut out = Vec::new();
    let cast = cast_ident(nonce);
    for a in &sig.inputs {
        if let syn::FnArg::Typed(pt) = a {
            let id = match &*pt.pat {
                syn::Pat::Ident(pi) => &pi.ident,
                _ => {
                    return Err(syn::Error::new(
                        pt.span(),
                        "#[decycle(structural)]: only simple identifier parameters are supported",
                    ))
                }
            };
            let ty = &pt.ty;
            if mentions_self(ty) {
                let dst = subst_self(ty, target);
                out.push(quote! { unsafe { #cast::<#ty, #dst>(#id) } });
            } else {
                out.push(quote!(#id));
            }
        }
    }
    Ok(out)
}

/// Rewrite every non-trivial parameter pattern to a fresh ident so the dispatch can forward each
/// param by name AND the fresh signature is legal in the bodiless `__run` trait declaration. Returns
/// the rewritten signature plus the `let <original-pattern> = <fresh-ident>;` rebinds to prepend to
/// the body (which still uses the original bindings). Only a bare `ident: T` (no `mut`, no `ref`, no
/// subpattern) is left alone — a `mut x` / `ref x` / destructured param must be rewritten too, else it
/// lands verbatim in the bodiless trait method decl and trips `E0642` (patterns in a fn without a
/// body) / the deny-by-default `patterns_in_fns_without_body` lint (for `mut`).
fn normalize_sig(sig: &syn::Signature, nonce: u64) -> (syn::Signature, Vec<TokenStream>) {
    let mut renamed = sig.clone();
    let mut rebinds = Vec::new();
    for (i, input) in renamed.inputs.iter_mut().enumerate() {
        if let syn::FnArg::Typed(pt) = input {
            let is_plain_ident = matches!(&*pt.pat,
                syn::Pat::Ident(pi)
                    if pi.subpat.is_none() && pi.by_ref.is_none() && pi.mutability.is_none());
            if !is_plain_ident {
                let fresh = arg_ident(i, nonce);
                let orig = (*pt.pat).clone();
                rebinds.push(quote! { let #orig = #fresh; });
                pt.pat = Box::new(parse_quote!(#fresh));
            }
        }
    }
    (renamed, rebinds)
}

/// Whether `ty` mentions the `Self` type anywhere.
fn mentions_self(ty: &Type) -> bool {
    let mut found = false;
    walk_type(ty, &mut |id| {
        if id == "Self" {
            found = true;
        }
    });
    found
}

/// A cyclic `where`-predicate that was stripped and whose bounded type is a *wrapped* member
/// (`Box<Stmt>`, `Vec<Stmt>`, …) — carrying the container and the trait it was bounded by.
pub(crate) struct StrippedWrapped {
    pub container: Type,
    pub trait_path: syn::Path,
}

/// Split an impl's generics into two where-clause variants + the wrapped stripped bounds:
///  * `reduced` — every cyclic `where`-pred dropped (used for the terminator + natural impls, so the
///    obligation cycle is broken);
///  * `local` — only the *wrapped* cyclic preds dropped; a **bare-member** cyclic pred (`B: Frob<N>`)
///    is KEPT (used for the local `__DecycleBody` impl). It resolves on-sight through the natural
///    impl (no E0275) and pins otherwise-uninferable generics for the body (e.g. `N` in `B.frob(n)`).
///  * `wrapped` — the wrapped stripped bounds, for forwarding assertions (kept OUT of `local` so a
///    `Deref`-reached wrapped bound needn't be provable).
fn reduce_generics(
    generics: &syn::Generics,
    self_ty: &Type,
    scc: &Scc,
    model: &Model,
) -> (syn::Generics, syn::Generics, Vec<StrippedWrapped>) {
    let adt_names: HashSet<String> = model.adts.keys().cloned().collect();
    let member_set: HashSet<String> = scc.types().into_iter().collect();
    let mut reduced = generics.clone();
    let mut local = generics.clone();
    let mut wrapped = Vec::new();

    let mut reduced_preds: syn::punctuated::Punctuated<WherePredicate, syn::Token![,]> =
        syn::punctuated::Punctuated::new();
    let mut local_preds: syn::punctuated::Punctuated<WherePredicate, syn::Token![,]> =
        syn::punctuated::Punctuated::new();

    for pred in generics.where_clause.iter().flat_map(|w| &w.predicates) {
        let WherePredicate::Type(pt) = pred else {
            // lifetime / eq predicates are never cyclic — keep verbatim in both.
            reduced_preds.push(pred.clone());
            local_preds.push(pred.clone());
            continue;
        };
        // Resolve `Self` in the bounded type to the impl's self type, so `where Self: Tr` (and
        // `where Box<Self>: Tr`) is detected, classified, and re-emitted exactly like `where A: Tr`:
        // the SCC graph and the forwarding assertion key on the concrete ADT, which `Self` alone
        // never matches. A bound with no `Self` is unchanged.
        let pt_owned = {
            let mut p = pt.clone();
            p.bounded_ty = subst_self(&p.bounded_ty, self_ty);
            p
        };
        let pt = &pt_owned;
        let refs = local_refs(&pt.bounded_ty, &adt_names);
        // Split THIS predicate's bounds: a *cyclic* bound is a trait `TrB` where a referenced type
        // `B` makes `(B, TrB)` a member of THIS SCC (possibly a different trait — that is what breaks
        // a cross-trait cycle). Only cyclic bounds break the obligation cycle; any co-bound on the
        // same predicate (`Clone` in `Expr: Eval + Clone`) must SURVIVE onto the reduced impls, else
        // valid input fails to compile.
        let mut cyclic_paths: Vec<syn::Path> = Vec::new();
        let mut noncyclic: syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]> =
            syn::punctuated::Punctuated::new();
        for b in &pt.bounds {
            let is_cyclic_bound = matches!(b,
                syn::TypeParamBound::Trait(tb) if tb.path.segments.last().is_some_and(|s|
                    refs.iter().any(|r| scc.contains(r, &s.ident.to_string()))));
            match b {
                syn::TypeParamBound::Trait(tb) if is_cyclic_bound => cyclic_paths.push(tb.path.clone()),
                _ => noncyclic.push(b.clone()),
            }
        }
        if cyclic_paths.is_empty() {
            // no cyclic bound: keep the whole (Self-resolved) predicate in both.
            reduced_preds.push(WherePredicate::Type(pt.clone()));
            local_preds.push(WherePredicate::Type(pt.clone()));
            continue;
        }
        // The surviving (non-cyclic) bounds rebuilt as their own predicate, if any remain.
        let survivor = (!noncyclic.is_empty()).then(|| {
            let mut keep = pt.clone();
            keep.bounds = noncyclic.clone();
            WherePredicate::Type(keep)
        });
        if is_wrapped_member(&pt.bounded_ty, &member_set) {
            // wrapped cyclic bound(s): dropped from both impls; each gets a forwarding assertion. The
            // co-bounds survive onto both.
            for trait_path in cyclic_paths {
                wrapped.push(StrippedWrapped {
                    container: pt.bounded_ty.clone(),
                    trait_path,
                });
            }
            if let Some(s) = &survivor {
                reduced_preds.push(s.clone());
                local_preds.push(s.clone());
            }
        } else {
            // bare cyclic bound: dropped from `reduced` (breaks the cycle) but the FULL (Self-resolved)
            // predicate is kept in `local` so the bare bound still pins generics on-sight through the
            // natural impl.
            if let Some(s) = survivor {
                reduced_preds.push(s);
            }
            local_preds.push(WherePredicate::Type(pt.clone()));
        }
    }

    finish_where(&mut reduced, reduced_preds);
    finish_where(&mut local, local_preds);
    (reduced, local, wrapped)
}

fn finish_where(
    g: &mut syn::Generics,
    preds: syn::punctuated::Punctuated<WherePredicate, syn::Token![,]>,
) {
    if preds.is_empty() {
        g.where_clause = None;
    } else if let Some(wc) = &mut g.where_clause {
        wc.predicates = preds;
    }
}

/// Is `ty` a *container around* a member (not the bare member itself)? `Box<Stmt>` → yes, `Stmt` → no.
fn is_wrapped_member(ty: &Type, members: &HashSet<String>) -> bool {
    match ty {
        Type::Path(tp) if tp.qself.is_none() => {
            let head = tp.path.segments.last().map(|s| s.ident.to_string());
            !head.is_some_and(|h| members.contains(&h))
        }
        _ => true, // tuple / array / reference / … are always containers
    }
}

/// Replace every member-headed path in `ty` with the ident `__DecycleX` (dropping its args), so
/// `Box<Stmt<T>>` becomes `Box<__DecycleX>` — the generic container for the forwarding assertion.
fn generalize_members(ty: &Type, members: &HashSet<String>) -> Type {
    let x = memberx_ident();
    let repl: Type = parse_quote!(#x);
    fn go(ty: &Type, members: &HashSet<String>, repl: &Type) -> Type {
        match ty {
            Type::Path(tp) if tp.qself.is_none() => {
                let head = tp.path.segments.last().map(|s| s.ident.to_string());
                if head.is_some_and(|h| members.contains(&h)) {
                    return repl.clone();
                }
                let mut tp = tp.clone();
                for seg in tp.path.segments.iter_mut() {
                    if let PathArguments::AngleBracketed(ab) = &mut seg.arguments {
                        for a in ab.args.iter_mut() {
                            if let GenericArgument::Type(t) = a {
                                *t = go(t, members, repl);
                            }
                        }
                    }
                }
                Type::Path(tp)
            }
            Type::Reference(r) => {
                let mut r = r.clone();
                r.elem = Box::new(go(&r.elem, members, repl));
                Type::Reference(r)
            }
            Type::Ptr(p) => {
                let mut p = p.clone();
                p.elem = Box::new(go(&p.elem, members, repl));
                Type::Ptr(p)
            }
            Type::Tuple(t) => {
                let mut t = t.clone();
                t.elems = t.elems.iter().map(|e| go(e, members, repl)).collect();
                Type::Tuple(t)
            }
            Type::Array(a) => {
                let mut a = a.clone();
                a.elem = Box::new(go(&a.elem, members, repl));
                Type::Array(a)
            }
            Type::Slice(s) => {
                let mut s = s.clone();
                s.elem = Box::new(go(&s.elem, members, repl));
                Type::Slice(s)
            }
            Type::Paren(p) => {
                let mut p = p.clone();
                p.elem = Box::new(go(&p.elem, members, repl));
                Type::Paren(p)
            }
            Type::Group(g) => {
                let mut g = g.clone();
                g.elem = Box::new(go(&g.elem, members, repl));
                Type::Group(g)
            }
            other => other.clone(),
        }
    }
    go(ty, members, &repl)
}

/// The forwarding assertion for a stripped wrapped predicate `Container<Member>: Trait`: a private
/// `const _` block that type-checks `for<__DecycleElem: Trait> Container<__DecycleElem>: Trait`
/// (encoded via two helper fns, since HRTBs can't carry trait bounds). If it fails, the user stripped
/// a container bound whose container does not forward the trait — a real soundness signal. Skipped
/// when the trait or generalized container still mentions an impl generic (would be out of the
/// const's scope) — the assertion stays a pure crate-private item, so no `private_interfaces` arises.
///
/// All items here are LOCAL to the `const _ {}` block, so they carry no hygiene nonce — a nonce would
/// only clutter the failing-bound diagnostic. The requirement fn is named as a sentence so rustc's
/// `... required by a bound in <name>` note reads as guidance, and the placeholder is `__DecycleElem`
/// so the message reads `the trait bound Box<__DecycleElem>: Tr is not satisfied`. The primary span
/// lands on the user's own `Container<Member>` bound (its tokens are cloned verbatim).
fn emit_forwarding_assertion(sw: &StrippedWrapped, scc: &Scc, reduced: &syn::Generics) -> TokenStream {
    let member_set: HashSet<String> = scc.types().into_iter().collect();
    let generalized = generalize_members(&sw.container, &member_set);
    let trait_path = &sw.trait_path;

    let param_idents: HashSet<String> = reduced
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(t) => Some(t.ident.to_string()),
            GenericParam::Const(c) => Some(c.ident.to_string()),
            _ => None,
        })
        .collect();
    if path_mentions(trait_path, &param_idents) || type_mentions(&generalized, &param_idents) {
        return quote!();
    }

    // Named so the failing bound reads as an explanation:
    //   error[E0277]: the trait bound `Box<__DecycleElem>: Tr` is not satisfied
    //   note: required by a bound in `stripped_wrapped_bound_needs_its_container_to_forward_the_trait`
    let needs = format_ident!("stripped_wrapped_bound_needs_its_container_to_forward_the_trait");
    let assert = format_ident!("__decycle_check_wrapped_bound_forwards");
    let u = memberx_ident();
    let x = memberx_ident();
    quote! {
        const _: () = {
            #[allow(dead_code, non_snake_case)]
            fn #needs<#u: #trait_path>() {}
            #[allow(dead_code, non_snake_case)]
            fn #assert<#x: #trait_path>() {
                #needs::<#generalized>()
            }
        };
    }
}

fn type_mentions(ty: &Type, idents: &HashSet<String>) -> bool {
    let mut found = false;
    walk_type(ty, &mut |id| {
        if idents.contains(&id.to_string()) {
            found = true;
        }
    });
    found
}

fn path_mentions(path: &syn::Path, idents: &HashSet<String>) -> bool {
    let mut found = false;
    for seg in &path.segments {
        if idents.contains(&seg.ident.to_string()) {
            found = true;
        }
        if let PathArguments::AngleBracketed(ab) = &seg.arguments {
            for a in &ab.args {
                if let GenericArgument::Type(t) = a {
                    if type_mentions(t, idents) {
                        found = true;
                    }
                }
            }
        }
    }
    found
}

// Generics rendering (`params_decl`/`params_use`/`wrap_angle`) lives in `crate::generics_fmt`,
// shared with the ranked engine.
