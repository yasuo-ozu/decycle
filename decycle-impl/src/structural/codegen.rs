//! Code generation: the per-member `#[repr(transparent)]` terminator `__MTerm`, the trait impl on it
//! (body via the trait-def-inside-body pattern), and the natural type's delegating impl.

use super::*;

/// An expression that reinterprets the receiver `self` into `target`-space (a same-layout type).
/// `&self`/`&mut self` use a pointer cast; any other receiver — owned `self`, or a custom type such as
/// `self: Box<Self>` / `Pin<&mut Self>` / `Rc<Self>` — casts the whole receiver value from its declared
/// type to that type with `Self` replaced by `target` (both are same-layout because `__MTerm` is a
/// `#[repr(transparent)]` wrapper of the natural type).
fn receiver_cast(r: &syn::Receiver, target: &Type) -> TokenStream {
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
        quote! { unsafe { __decycle_cast::<#src, #dst>(#self_tok) } }
    }
}

/// Replace every `Self` type occurrence in `ty` with `replacement`.
fn subst_self(ty: &Type, replacement: &Type) -> Type {
    match ty {
        Type::Path(tp) if tp.qself.is_none() && tp.path.is_ident("Self") => replacement.clone(),
        Type::Path(tp) => {
            let mut tp = tp.clone();
            for seg in tp.path.segments.iter_mut() {
                if let PathArguments::AngleBracketed(ab) = &mut seg.arguments {
                    for a in ab.args.iter_mut() {
                        if let GenericArgument::Type(t) = a {
                            *t = subst_self(t, replacement);
                        }
                    }
                }
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
        other => other.clone(),
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
pub(crate) fn make_term_item(member: &Adt) -> TokenStream {
    let term = term_ident(&member.ident.to_string());
    let m_id = &member.ident;
    let vis = member.vis();
    let decl = bare_generics_decl(&member.generics);
    let uses = bare_generics_use(&member.generics);
    let decl_angle = wrap_angle(&decl);
    let use_angle = wrap_angle(&uses);
    quote! {
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
        let term = term_ident(&m);
        (parse_quote!(#m_ident), parse_quote!(#term))
    } else {
        let term = term_ident(&m);
        (
            parse_quote!(#m_ident< #(#self_args),* >),
            parse_quote!(#term< #(#self_args),* >),
        )
    };

    // `reduced` (all cyclic preds dropped) drives the terminator + natural impls; `local` (only wrapped
    // cyclic preds dropped) drives each method's local `__DecycleBody` impl so bare cyclic bounds still
    // pin generics for the body. `stripped_wrapped` → forwarding assertions.
    let (reduced, local, stripped_wrapped) = reduce_generics(&im.item.generics, scc, model);
    let (impl_g, _, where_g) = reduced.split_for_impl();

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
            term_methods.extend(rec_method(f, &natural, &local, &assoc_items)?);
            nat_methods.extend(nat_method(f, &term_ty, &trait_path)?);
        }
    }

    Ok(quote! {
        #assertions
        impl #impl_g #trait_path for #term_ty #where_g {
            #term_methods
        }
        impl #impl_g #trait_path for #natural #where_g {
            #nat_methods
        }
    })
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
) -> syn::Result<TokenStream> {
    let attrs = &f.attrs;
    let vis = &f.vis;
    let outer_sig = &f.sig;
    let body = &f.block;

    // `fn __run(<original sig>)` — same signature, renamed.
    let mut run_sig = f.sig.clone();
    run_sig.ident = format_ident!("__run");

    let (impl_g, _, where_g) = reduced.split_for_impl();
    let trait_g = wrap_angle(&bare_generics_decl(reduced)); // `<Span, Token>` for the trait decl
    let use_g = wrap_angle(&bare_generics_use(reduced)); // `<Span, Token>` at the impl/call

    // associated-item decls (for the local trait) and defs (for the local impl), so a `Self::Assoc`
    // in the body resolves against the natural type.
    let assoc_decls: Vec<TokenStream> = assoc_items.iter().map(|it| assoc_decl(it)).collect();
    let assoc_defs: Vec<TokenStream> = assoc_items.iter().map(|it| it.to_token_stream()).collect();

    // dispatch: cast receiver + any Self-typed args into natural-space, forward the rest, call __run,
    // cast result up.
    let mut call_args: Vec<TokenStream> = Vec::new();
    if let Some(r) = outer_sig.receiver() {
        call_args.push(receiver_cast(r, natural));
    }
    call_args.extend(forward_args(outer_sig, natural)?);
    let turbofish = method_turbofish(outer_sig);
    let call = quote! {
        < #natural as __DecycleBody #use_g >::__run #turbofish ( #(#call_args),* )
    };
    let dispatch = match &outer_sig.output {
        ReturnType::Default => quote! { #call; },
        ReturnType::Type(..) => quote! { unsafe { __decycle_cast(#call) } },
    };

    Ok(quote! {
        #(#attrs)* #vis #outer_sig {
            trait __DecycleBody #trait_g : Sized {
                #(#assoc_decls)*
                #run_sig ;
            }
            impl #impl_g __DecycleBody #use_g for #natural #where_g {
                #(#assoc_defs)*
                #run_sig #body
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

/// Delegating method on the natural type: cast the receiver (if any) into `Rec`-space, call the Rec
/// impl's trait method, and cast the result back.
fn nat_method(
    f: &syn::ImplItemFn,
    closed_rec: &Type,
    trait_path: &syn::Path,
) -> syn::Result<TokenStream> {
    let attrs = &f.attrs;
    let vis = &f.vis;
    let sig = &f.sig;
    let name = &sig.ident;
    let mut call_args: Vec<TokenStream> = Vec::new();
    if let Some(r) = sig.receiver() {
        call_args.push(receiver_cast(r, closed_rec));
    }
    call_args.extend(forward_args(sig, closed_rec)?);
    // forward the method's own type/const generics (lifetimes inferred) so non-inferable params resolve
    let turbofish = method_turbofish(sig);
    let call = quote! { < #closed_rec as #trait_path >::#name #turbofish ( #(#call_args),* ) };
    match &sig.output {
        ReturnType::Default => Ok(quote! { #(#attrs)* #vis #sig { #call; } }),
        ReturnType::Type(..) => Ok(quote! {
            #(#attrs)* #vis #sig { unsafe { __decycle_cast(#call) } }
        }),
    }
}

/// Forward the non-receiver parameters (receiver handled separately). An argument whose type mentions
/// `Self` (e.g. `other: &Self` in `PartialEq::eq`) is cast into `target`-space, exactly like the
/// receiver; others pass through by ident.
fn forward_args(sig: &syn::Signature, target: &Type) -> syn::Result<Vec<TokenStream>> {
    let mut out = Vec::new();
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
                out.push(quote! { unsafe { __decycle_cast::<#ty, #dst>(#id) } });
            } else {
                out.push(quote!(#id));
            }
        }
    }
    Ok(out)
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
        let mut is_cyclic = false;
        let mut is_wrapped = false;
        if let WherePredicate::Type(pt) = pred {
            let refs = local_refs(&pt.bounded_ty, &adt_names);
            // A cyclic edge: some trait bound `TrB` on the pred where a referenced type `B` makes
            // `(B, TrB)` a member of THIS SCC (possibly a different trait — that's what breaks a
            // cross-trait cycle).
            let trait_bound = pt.bounds.iter().find_map(|b| match b {
                syn::TypeParamBound::Trait(tb) => {
                    let btr = tb.path.segments.last()?.ident.to_string();
                    refs.iter()
                        .any(|r| scc.contains(r, &btr))
                        .then(|| tb.path.clone())
                }
                _ => None,
            });
            if let Some(trait_path) = trait_bound {
                is_cyclic = true;
                if is_wrapped_member(&pt.bounded_ty, &member_set) {
                    is_wrapped = true;
                    wrapped.push(StrippedWrapped {
                        container: pt.bounded_ty.clone(),
                        trait_path,
                    });
                }
            }
        }
        match (is_cyclic, is_wrapped) {
            (false, _) => {
                reduced_preds.push(pred.clone());
                local_preds.push(pred.clone());
            }
            (true, false) => local_preds.push(pred.clone()), // bare cyclic: kept for the local impl
            (true, true) => {}                               // wrapped cyclic: dropped everywhere
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
            !head.map_or(false, |h| members.contains(&h))
        }
        _ => true, // tuple / array / reference / … are always containers
    }
}

/// Replace every member-headed path in `ty` with the ident `__DecycleX` (dropping its args), so
/// `Box<Stmt<T>>` becomes `Box<__DecycleX>` — the generic container for the forwarding assertion.
fn generalize_members(ty: &Type, members: &HashSet<String>) -> Type {
    let repl: Type = parse_quote!(__DecycleX);
    fn go(ty: &Type, members: &HashSet<String>, repl: &Type) -> Type {
        match ty {
            Type::Path(tp) if tp.qself.is_none() => {
                let head = tp.path.segments.last().map(|s| s.ident.to_string());
                if head.map_or(false, |h| members.contains(&h)) {
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
/// `const _` block that type-checks `for<__DecycleX: Trait> Container<__DecycleX>: Trait`
/// (encoded via two helper fns, since HRTBs can't carry trait bounds). If it fails, the user stripped a
/// container bound whose container does not forward the trait — a real soundness signal. Skipped when
/// the trait or generalized container still mentions an impl generic (would be out of the const's
/// scope) — the assertion stays a pure crate-private item, so no `private_interfaces` ever arises.
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

    quote! {
        const _: () = {
            #[allow(dead_code)]
            fn __decycle_needs<__DecycleU: #trait_path>() {}
            #[allow(dead_code)]
            fn __decycle_assert<__DecycleX: #trait_path>() {
                __decycle_needs::<#generalized>()
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

// ---- generics rendering helpers ----

/// Param declarations without bounds: `'a, Span, const N: usize`.
pub(crate) fn bare_generics_decl(g: &syn::Generics) -> TokenStream {
    let parts = g.params.iter().map(|p| match p {
        GenericParam::Lifetime(l) => {
            let lt = &l.lifetime;
            quote!(#lt)
        }
        GenericParam::Type(t) => {
            let id = &t.ident;
            quote!(#id)
        }
        GenericParam::Const(c) => {
            let id = &c.ident;
            let ty = &c.ty;
            quote!(const #id: #ty)
        }
    });
    quote!( #(#parts),* )
}

/// Param uses: `'a, Span, N`.
pub(crate) fn bare_generics_use(g: &syn::Generics) -> TokenStream {
    let parts = g.params.iter().map(|p| match p {
        GenericParam::Lifetime(l) => {
            let lt = &l.lifetime;
            quote!(#lt)
        }
        GenericParam::Type(t) => {
            let id = &t.ident;
            quote!(#id)
        }
        GenericParam::Const(c) => {
            let id = &c.ident;
            quote!(#id)
        }
    });
    quote!( #(#parts),* )
}

/// Wrap a comma-list in angle brackets, or empty if the list is empty.
pub(crate) fn wrap_angle(inner: &TokenStream) -> TokenStream {
    if inner.is_empty() {
        quote!()
    } else {
        quote!( < #inner > )
    }
}
