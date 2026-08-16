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
            // A `Self` in qself position (`<Self as Tr>::Out`) is substituted too. (Callers resolve
            // `Self`-rooted projections via `normalize_projections` FIRST, so by the time a type
            // reaches this substitution no unresolvable projection remains — this arm is
            // belt-and-braces for the resolvable leftovers.)
            if let Some(q) = &mut tp.qself {
                q.ty = Box::new(subst_self(&q.ty, replacement));
            }
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

/// `#[repr(transparent)] <vis> struct __MTerm<params>(<vis> M<params>) <where M's own clause>;` — the
/// per-member terminator, a same-layout wrapper of the natural type. Its visibility (and its field's)
/// matches `M`, so wrapping a private cycle type doesn't expose it through a `pub` interface (no
/// `private_interfaces`).
///
/// The wrapped type's predicates **come along, in both spellings**: inline bounds via
/// `DeclBounds::Keep` (`enum Expr<S: Span>`) and the `where`-clause verbatim (`where S: Clone`).
/// Without them a type whose own well-formedness needs a predicate gives a wrapper whose single field
/// is ill-formed — `E0277 … required by a bound in Expr`, reported against the *generated* struct and
/// therefore reading as if the caller's type were at fault. `Bare` is right for the impl generics
/// below (the impl re-states what it needs) but never for this wrapper, which must be exactly as
/// constrained as what it wraps.
pub(crate) fn make_term_item(member: &Adt, nonce: u64) -> TokenStream {
    let term = term_ident(&member.ident.to_string(), nonce);
    let m_id = &member.ident;
    let vis = member.vis();
    let decl = params_decl(&member.generics, DeclBounds::Keep);
    let uses = params_use(&member.generics);
    let decl_angle = wrap_angle(&decl);
    let use_angle = wrap_angle(&uses);
    // Replicate the type's `#[cfg]`s so a cfg-gated cyclic type's terminator strips with it (else the
    // terminator would wrap a type rustc removed → E0412).
    let cfgs = crate::extract_cfg_attrs(member.attrs());
    let where_clause = member.generics.where_clause.as_ref();
    quote! {
        #(#cfgs)*
        #[repr(transparent)]
        #[allow(dead_code)]
        #vis struct #term #decl_angle ( #vis #m_id #use_angle ) #where_clause ;
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
            term_methods.extend(rec_method(
                f,
                &natural,
                &local,
                &reduced,
                &assoc_items,
                &trait_path,
                model.nonce,
            )?);
            nat_methods.extend(nat_method(
                f,
                &term_ty,
                &trait_path,
                &assoc_items,
                model.nonce,
            )?);
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
    outer: &syn::Generics,
    assoc_items: &[&ImplItem],
    trait_path: &syn::Path,
    nonce: u64,
) -> syn::Result<TokenStream> {
    let attrs = &f.attrs;
    let vis = &f.vis;
    let ctx = ProjCtx::new(assoc_items, trait_path);
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
            // Resolve `Self::Out`-style projections first, so the check sees what the type MEANS
            // (`impl Fn(&Self::Out)` with `type Out = Self` is the unnameable shape; with
            // `type Out = i64` it is harmless). Unresolvable projections error here, up-front.
            let nty = normalize_projections(&pt.ty, &ctx, 0)?;
            if has_self_mentioning_impl_trait(&nty) {
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
    // the original patterns are rebound at the top of the body. A `mut self` receiver's `mut` is
    // stripped the same way (it is a pattern, illegal in the bodiless `__run` decl) and returned so
    // it can be re-applied on the body-holding impl fn below.
    let (norm_sig, rebinds, receiver_mut) = normalize_sig(&f.sig, nonce);
    let outer_sig = &norm_sig;
    // Splice the user body's STATEMENTS (not the whole `{ .. }` block): re-wrapping the block as
    // `{ #body }` would double-brace a single-expression body (`{ { 1 } }`), tripping `unused_braces`
    // under `#![deny(warnings)]` / `clippy -D warnings`. (The ranked engine splices statements for the
    // same reason.) The tail expression stays the tail, so the return value is unchanged.
    let body_stmts = &f.block.stmts;

    let body_tr = body_ident(nonce);
    let run = run_ident(nonce);

    // `fn __run(<normalized sig>)` — same signature, renamed. Its parameter/return types get their
    // `Self` projections resolved too: a verbatim `<Self as Tr>::Out` would need a `Self: Tr` bound
    // the local `__DecycleBody` trait deliberately does not have, and the resolved form is what the
    // dispatch's cast target is computed from, so decl, impl and call site agree by construction.
    let mut run_sig = norm_sig.clone();
    run_sig.ident = run.clone();
    for input in run_sig.inputs.iter_mut() {
        if let syn::FnArg::Typed(pt) = input {
            pt.ty = Box::new(normalize_projections(&pt.ty, &ctx, 0)?);
        }
    }
    if let ReturnType::Type(_, t) = &mut run_sig.output {
        *t = Box::new(normalize_projections(t, &ctx, 0)?);
    }
    // The body-holding impl fn carries the user's `mut self` back (stripped from the decl above) so
    // the body's mutation of `self` still compiles; a by-value receiver's binding mode is free to
    // differ between a trait decl and its impl.
    let mut run_sig_impl = run_sig.clone();
    if let Some(m) = receiver_mut {
        if let Some(syn::FnArg::Receiver(r)) = run_sig_impl.inputs.first_mut() {
            r.mutability = Some(m);
        }
    }

    let (impl_g, _, where_g) = reduced.split_for_impl();
    let trait_g = wrap_angle(&params_decl(reduced, DeclBounds::Bare)); // `<Span, Token>` for the trait decl
    let trait_where = body_trait_where(outer);
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
    call_args.extend(forward_args(outer_sig, natural, &ctx, nonce)?);
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
            trait #body_tr #trait_g : Sized #trait_where {
                #(#assoc_decls)*
                #run_sig ;
            }
            impl #impl_g #body_tr #use_g for #natural #where_g {
                #(#assoc_defs)*
                // `run_sig_impl`, not `run_sig`: this copy carries the user's `mut self` back.
                // The trait decl above must not have it (a receiver pattern is illegal in a
                // bodiless fn), but the body below may well assign through `self`.
                #(#prop)* #run_sig_impl { #(#rebinds)* #(#body_stmts)* }
            }
            #dispatch
        }
    })
}

/// The `where`-clause the body-holding local trait's **declaration** needs.
///
/// The declaration's generic params are rendered [`Bare`](DeclBounds::Bare) — every bound dropped —
/// because a param bound may itself be the cyclic one, and re-stating it at the declaration would put
/// the cycle straight back. But dropping *everything* is too much: `__run`'s signature is copied from
/// the user's method, so if it projects through a param (`-> ParseError<<Atom as Spanned>::Span>`) the
/// declaration is ill-formed on its own (E0277 `Atom: Spanned` is not satisfied) even though every impl
/// and the single call site can prove it.
///
/// So re-state the predicates of the **reduced** generics — the ones already on the enclosing
/// terminator impl, which is where the only call to `__run` sits, so each is provable there by
/// construction. They are cycle-free by definition (`reduce_generics` dropped the cyclic bounds), and
/// the body impl is driven by `local`, which keeps a superset of them, so it satisfies the declaration.
///
/// Predicates whose *bounds* mention `Self` are skipped: `Self` means the terminator inside the
/// enclosing impl but the natural type at the declaration, so re-stating one would change what it
/// asserts. (`reduce_generics` already substituted `Self` out of every *bounded type*.)
fn body_trait_where(outer: &syn::Generics) -> TokenStream {
    let preds: Vec<&WherePredicate> = outer
        .where_clause
        .iter()
        .flat_map(|w| &w.predicates)
        .filter(|p| match p {
            WherePredicate::Type(pt) => !pt.bounds.iter().any(|b| match b {
                syn::TypeParamBound::Trait(tb) => tb
                    .path
                    .segments
                    .iter()
                    .any(|s| s.ident == "Self" || args_mention_self(&s.arguments)),
                _ => false,
            }),
            _ => true,
        })
        .collect();
    if preds.is_empty() {
        quote!()
    } else {
        quote!(where #(#preds),*)
    }
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
    assoc_items: &[&ImplItem],
    nonce: u64,
) -> syn::Result<TokenStream> {
    let attrs = &f.attrs;
    let vis = &f.vis;
    // Same projection context as the terminator side, so both classify `Self::Out` identically —
    // if they disagreed, one side would cast an argument the other forwards verbatim.
    let ctx = ProjCtx::new(assoc_items, trait_path);
    // The receiver's `mut` is dropped: this natural method only forwards, it never mutates
    // `self`, and the binding mode of a by-value receiver is local to each fn.
    let (norm_sig, _, _) = normalize_sig(&f.sig, nonce);
    let sig = &norm_sig;
    let name = &sig.ident;
    let mut call_args: Vec<TokenStream> = Vec::new();
    if let Some(r) = sig.receiver() {
        call_args.push(receiver_cast(r, closed_rec, nonce));
    }
    call_args.extend(forward_args(sig, closed_rec, &ctx, nonce)?);
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

/// See through `(T)` and the invisible groups a macro-expanded type can be wrapped in, so the
/// trait-object checks below match on the real shape.
fn strip_group(ty: &Type) -> &Type {
    match ty {
        Type::Group(g) => strip_group(&g.elem),
        Type::Paren(p) => strip_group(&p.elem),
        other => other,
    }
}

/// Re-wrap a `&dyn Fn(..)`-shaped argument that mentions `Self` into `target`-space.
///
/// The type-punning cast is unsound for trait objects: `dyn Fn(&A) -> R` and `dyn Fn(&ATerm) -> R`
/// are *different traits*, so `transmute`-ing the wide pointer leaves it carrying a vtable that
/// names the wrong one (miri: "wrong trait in wide pointer vtable"). Both are pointer-pair sized,
/// so `__DecycleSizeGuard` cannot see it.
///
/// Instead of punning, build a fresh closure whose parameters are already in `target`-space and
/// which casts each argument back before calling the user's object. `&closure` then coerces to
/// `&dyn Fn(..)` the ordinary way, so the vtable is produced by the compiler rather than forged.
/// The closure is a temporary living to the end of the forwarding call, so this costs no
/// allocation.
///
/// Returns `Ok(None)` when `ty` is not a trait object at all (the caller's plain cast is correct
/// for those — a bare `fn(&Self)` pointer is ABI-compatible and stays punned). Shapes that are
/// trait objects but cannot be rebuilt this way are rejected rather than silently punned.
fn dyn_fn_adapter(
    ty: &Type,
    target: &Type,
    id: &syn::Ident,
    nonce: u64,
) -> syn::Result<Option<TokenStream>> {
    // Only `&dyn ..` is in scope here; `Box<dyn ..>`/`&mut dyn ..` fall through to the reject
    // below via `mentions_self` on their inner object.
    let Type::Reference(r) = ty else {
        return match strip_group(ty) {
            Type::TraitObject(_) => Err(syn::Error::new(
                ty.span(),
                "#[decycle(structural)]: a by-value or boxed trait object mentioning `Self` \
                 cannot be forwarded soundly (its vtable names a different trait once `Self` is \
                 substituted). Take it by shared reference (`&dyn Fn(&Self) -> _`), or use a \
                 generic parameter instead of a trait object.",
            )),
            _ => Ok(None),
        };
    };
    let Type::TraitObject(to) = strip_group(&r.elem) else {
        return Ok(None);
    };
    if r.mutability.is_some() {
        return Err(syn::Error::new(
            ty.span(),
            "#[decycle(structural)]: `&mut dyn ..` mentioning `Self` cannot be forwarded soundly \
             (its vtable names a different trait once `Self` is substituted). Take it by shared \
             reference (`&dyn Fn(&Self) -> _`), or use a generic parameter instead.",
        ));
    }
    // Exactly one trait bound, spelled with `Fn(..)` sugar. Extra trait bounds (`+ Send`) are
    // refused because the rebuilt closure would have to prove them itself.
    let mut fn_bound = None;
    for b in &to.bounds {
        match b {
            syn::TypeParamBound::Lifetime(_) => {}
            syn::TypeParamBound::Trait(tb) => {
                if fn_bound.is_some() {
                    fn_bound = None;
                    break;
                }
                fn_bound = Some(tb);
            }
            _ => {
                fn_bound = None;
                break;
            }
        }
    }
    let sugar = fn_bound.and_then(|tb| {
        let seg = tb.path.segments.last()?;
        match (&seg.arguments, seg.ident.to_string().as_str()) {
            (PathArguments::Parenthesized(pa), "Fn") => Some(pa),
            _ => None,
        }
    });
    let Some(pa) = sugar else {
        return Err(syn::Error::new(
            ty.span(),
            "#[decycle(structural)]: only `&dyn Fn(..)` trait objects mentioning `Self` can be \
             forwarded. `FnMut`/`FnOnce`, additional bounds such as `+ Send`, and user traits \
             mentioning `Self` would need a vtable for a trait that does not exist after `Self` \
             is substituted. Use a generic parameter (`F: FnMut(&Self)`) instead.",
        ));
    };

    let cast = cast_ident(nonce);
    let mut params = Vec::new();
    let mut args = Vec::new();
    for (i, src_ty) in pa.inputs.iter().enumerate() {
        let p = syn::Ident::new(&format!("__dcl_adapt_{i}_{nonce:x}"), Span::call_site());
        let dst_ty = subst_self(src_ty, target);
        // The adapter is declared in target-space and casts each argument back before handing it
        // to the user's closure, which still expects natural-space types.
        if mentions_self(src_ty) {
            args.push(quote! { unsafe { #cast::<#dst_ty, #src_ty>(#p) } });
        } else {
            args.push(quote! { #p });
        }
        params.push(quote! { #p: #dst_ty });
    }
    let call = quote! { #id(#(#args),*) };
    // A `Self`-mentioning return travels the other way: natural-space result out to target-space.
    let body = match &pa.output {
        syn::ReturnType::Type(_, rt) if mentions_self(rt) => {
            let dst_rt = subst_self(rt, target);
            quote! { unsafe { #cast::<#rt, #dst_rt>(#call) } }
        }
        _ => call,
    };
    Ok(Some(quote! { &move |#(#params),*| #body }))
}

/// Forward the non-receiver parameters (receiver handled separately). An argument whose type mentions
/// `Self` (e.g. `other: &Self` in `PartialEq::eq`) is cast into `target`-space, exactly like the
/// receiver; others pass through by ident.
///
/// Types are first run through [`normalize_projections`], so a parameter typed via a `Self`
/// projection is classified by what the projection RESOLVES to: `s: Self::Out` with
/// `type Out = Self` is cast (as `Self`), with `type Out = i64` it is forwarded untouched — never
/// cast on the strength of the spelling alone.
fn forward_args(
    sig: &syn::Signature,
    target: &Type,
    ctx: &ProjCtx,
    nonce: u64,
) -> syn::Result<Vec<TokenStream>> {
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
            let ty = normalize_projections(&pt.ty, ctx, 0)?;
            if mentions_self(&ty) {
                // A trait object may NOT be punned: `dyn Fn(&A)` and `dyn Fn(&ATerm)` are
                // different traits, so transmuting the wide pointer keeps a vtable naming the
                // wrong one. They are the same size, so the size guard cannot catch it. Rebuild
                // the object by coercion instead — see `dyn_fn_adapter`.
                if let Some(adapter) = dyn_fn_adapter(&ty, target, id, nonce)? {
                    out.push(adapter);
                    continue;
                }
                // The same vtable forgery hides behind any other pointer shape (`Box<dyn ..>`,
                // `&&dyn ..`, `fn(&dyn ..)`); none of those can be rebuilt by coercion here, so
                // refuse them rather than pun them.
                if contains_self_dyn(&ty) {
                    return Err(syn::Error::new(
                        pt.ty.span(),
                        "#[decycle(structural)]: a trait object mentioning `Self` in this position \
                         cannot be forwarded soundly (its vtable would name a different trait once \
                         `Self` is substituted). Only a direct `&dyn Fn(..)` argument is rebuilt by \
                         coercion; use that, a `fn(..)` pointer, or a generic parameter instead.",
                    ));
                }
                let dst = subst_self(&ty, target);
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
///
/// The RECEIVER gets the same treatment: a by-value `mut self` (incl. `mut self: Box<Self>`) is a
/// binding-mode pattern, illegal in the bodiless `__run` decl for the same reason — but `self` cannot
/// be rebound with a `let`, so instead of a rebind the stripped `mut` token is RETURNED (third
/// element) and re-applied by the caller onto the body-holding impl fn only, where the user's body
/// still expects a mutable `self`. (`&mut self` has `reference` set and is untouched — there the
/// `mut` is part of the type, not a pattern.)
fn normalize_sig(
    sig: &syn::Signature,
    nonce: u64,
) -> (syn::Signature, Vec<TokenStream>, Option<syn::token::Mut>) {
    let mut renamed = sig.clone();
    let mut rebinds = Vec::new();
    let mut receiver_mut = None;
    for (i, input) in renamed.inputs.iter_mut().enumerate() {
        match input {
            syn::FnArg::Receiver(r) if r.reference.is_none() && r.mutability.is_some() => {
                receiver_mut = r.mutability.take();
            }
            syn::FnArg::Typed(pt) => {
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
            _ => {}
        }
    }
    (renamed, rebinds, receiver_mut)
}

/// Whether `ty` mentions the `Self` type anywhere. Unlike [`walk_type`] (which visits only a path's
/// LAST segment, the right granularity for ADT-name matching), this checks EVERY path segment plus
/// the `qself` position and associated-type bindings — so `Self::Out`, `<Self as Tr>::Out` and
/// `dyn Tr<Item = Self>` are all detected. Detection must err on the side of "mentions", because a
/// missed `Self` is forwarded un-cast (E0308 at best).
fn mentions_self(ty: &Type) -> bool {
    match ty {
        Type::Path(tp) => {
            tp.qself.as_ref().is_some_and(|q| mentions_self(&q.ty))
                || tp.path.segments.iter().any(|seg| {
                    seg.ident == "Self" || args_mention_self(&seg.arguments)
                })
        }
        Type::Reference(r) => mentions_self(&r.elem),
        Type::Ptr(p) => mentions_self(&p.elem),
        Type::Array(a) => mentions_self(&a.elem),
        Type::Slice(s) => mentions_self(&s.elem),
        Type::Paren(p) => mentions_self(&p.elem),
        Type::Group(g) => mentions_self(&g.elem),
        Type::Tuple(t) => t.elems.iter().any(mentions_self),
        Type::BareFn(bf) => {
            bf.inputs.iter().any(|i| mentions_self(&i.ty))
                || matches!(&bf.output, syn::ReturnType::Type(_, t) if mentions_self(t))
        }
        Type::TraitObject(to) => bounds_mention_self(&to.bounds),
        Type::ImplTrait(it) => bounds_mention_self(&it.bounds),
        _ => false,
    }
}

fn args_mention_self(args: &PathArguments) -> bool {
    match args {
        PathArguments::AngleBracketed(ab) => ab.args.iter().any(|a| match a {
            GenericArgument::Type(t) => mentions_self(t),
            GenericArgument::AssocType(at) => mentions_self(&at.ty),
            GenericArgument::Constraint(c) => c.bounds.iter().any(|b| match b {
                syn::TypeParamBound::Trait(tb) => {
                    tb.path.segments.iter().any(|s| args_mention_self(&s.arguments))
                }
                _ => false,
            }),
            _ => false,
        }),
        PathArguments::Parenthesized(p) => {
            p.inputs.iter().any(mentions_self)
                || matches!(&p.output, syn::ReturnType::Type(_, t) if mentions_self(t))
        }
        PathArguments::None => false,
    }
}

fn bounds_mention_self(
    bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
) -> bool {
    bounds.iter().any(|b| match b {
        syn::TypeParamBound::Trait(tb) => tb
            .path
            .segments
            .iter()
            .any(|s| s.ident == "Self" || args_mention_self(&s.arguments)),
        _ => false,
    })
}

/// Does `ty` contain a trait object whose bounds mention `Self` — in ANY position (`Box<dyn ..>`,
/// `&&dyn ..`, `fn(&dyn ..)`)? Such an object must never reach the plain layout cast: `dyn Fn(&A)`
/// and `dyn Fn(&ATerm)` are different traits, so punning the wide pointer (or a pointer leading to
/// it) leaves a vtable naming the wrong trait, and both sides are pointer-sized so the size guard
/// cannot see it. The one supported shape (a direct `&dyn Fn(..)` argument) is rebuilt by coercion
/// in [`dyn_fn_adapter`] before this check runs; everything else is rejected.
fn contains_self_dyn(ty: &Type) -> bool {
    match ty {
        Type::TraitObject(_) => mentions_self(ty),
        Type::Reference(r) => contains_self_dyn(&r.elem),
        Type::Ptr(p) => contains_self_dyn(&p.elem),
        Type::Array(a) => contains_self_dyn(&a.elem),
        Type::Slice(s) => contains_self_dyn(&s.elem),
        Type::Paren(p) => contains_self_dyn(&p.elem),
        Type::Group(g) => contains_self_dyn(&g.elem),
        Type::Tuple(t) => t.elems.iter().any(contains_self_dyn),
        Type::BareFn(bf) => {
            bf.inputs.iter().any(|i| contains_self_dyn(&i.ty))
                || matches!(&bf.output, syn::ReturnType::Type(_, t) if contains_self_dyn(t))
        }
        Type::Path(tp) => {
            tp.qself.as_ref().is_some_and(|q| contains_self_dyn(&q.ty))
                || tp.path.segments.iter().any(|seg| match &seg.arguments {
                    PathArguments::AngleBracketed(ab) => ab.args.iter().any(|a| match a {
                        GenericArgument::Type(t) => contains_self_dyn(t),
                        GenericArgument::AssocType(at) => contains_self_dyn(&at.ty),
                        _ => false,
                    }),
                    PathArguments::Parenthesized(p) => {
                        p.inputs.iter().any(contains_self_dyn)
                            || matches!(&p.output,
                                syn::ReturnType::Type(_, t) if contains_self_dyn(t))
                    }
                    PathArguments::None => false,
                })
        }
        _ => false,
    }
}

/// Recursion cap for [`normalize_projections`] — guards against a self-referential associated-type
/// definition (`type Out = Self::Out;`), which rustc itself rejects but must not hang the macro.
const MAX_PROJ_DEPTH: u8 = 16;

/// The impl's own associated-type definitions plus its trait name — the context in which a `Self`
/// projection in a method signature can be resolved at macro time.
struct ProjCtx {
    /// assoc-type name → (definition type, whether the assoc type has its own generics).
    assoc: BTreeMap<String, (Type, bool)>,
    /// Last-segment ident of the trait this impl implements (the crate-wide coherence key).
    trait_ident: String,
}

impl ProjCtx {
    fn new(assoc_items: &[&ImplItem], trait_path: &syn::Path) -> Self {
        let mut assoc = BTreeMap::new();
        for it in assoc_items {
            if let ImplItem::Type(t) = it {
                assoc.insert(
                    t.ident.to_string(),
                    (t.ty.clone(), !t.generics.params.is_empty()),
                );
            }
        }
        let trait_ident = trait_path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        ProjCtx { assoc, trait_ident }
    }
}

/// Is `ty` (through parens/groups) exactly the plain `Self` path?
fn is_plain_self(ty: &Type) -> bool {
    matches!(strip_group(ty),
        Type::Path(tp) if tp.qself.is_none() && tp.path.is_ident("Self"))
}

/// Resolve every `Self`-rooted associated-type projection in `ty` against the impl's OWN assoc-type
/// definitions, macro-side: `Self::Out` (and `<Self as Tr>::Out` for this impl's trait, and chains
/// like `Self::Out::Inner`) is replaced by the definition of `Out`, recursively. This is what makes
/// the cast decision correct — `type Out = Self` means the parameter must be cast into
/// terminator-space, `type Out = i64` means it must NOT be — and what keeps the `__DecycleBody`
/// `__run` signature legal (a verbatim `<Self as Tr>::Out` would demand a `Self: Tr` bound the local
/// trait deliberately does not have).
///
/// Projections that cannot be resolved here are rejected with an explanation rather than emitted
/// broken: `Self::X` where `X` is not defined in this impl (a supertrait's or another trait's assoc
/// type), `<Self as OtherTrait>::X`, a projection whose base mentions `Self` without being `Self`
/// (`<Box<Self> as Tr>::X`), and generic associated types.
fn normalize_projections(ty: &Type, ctx: &ProjCtx, depth: u8) -> syn::Result<Type> {
    if depth > MAX_PROJ_DEPTH {
        return Err(syn::Error::new(
            ty.span(),
            "#[decycle(structural)]: recursion limit while resolving `Self::..` associated-type \
             projections (self-referential associated type definition?)",
        ));
    }
    match ty {
        Type::Path(tp) => {
            // `Self::Out[::Inner..]` — a bare Self-rooted projection.
            if tp.qself.is_none()
                && tp.path.leading_colon.is_none()
                && tp.path.segments.len() >= 2
                && tp.path.segments[0].ident == "Self"
            {
                let segs: Vec<syn::PathSegment> =
                    tp.path.segments.iter().skip(1).cloned().collect();
                return resolve_self_projection(&segs, ty.span(), ctx, depth);
            }
            if let Some(q) = &tp.qself {
                let base = normalize_projections(&q.ty, ctx, depth + 1)?;
                if is_plain_self(&base) {
                    // `<Self as Tr>::Out[..]` (position ≥ 1) or `<Self>::Out[..]` (position 0).
                    if q.position >= 1 {
                        let tr_seg = &tp.path.segments[q.position - 1];
                        if tr_seg.ident != ctx.trait_ident {
                            return Err(syn::Error::new(
                                ty.span(),
                                format!(
                                    "#[decycle(structural)]: a `Self` projection through a \
                                     different trait (`<Self as {}>::..`) is not supported — the \
                                     generated terminator implements only this impl's trait, so \
                                     the projection would not resolve on it. Spell the concrete \
                                     type instead.",
                                    tr_seg.ident
                                ),
                            ));
                        }
                    }
                    let segs: Vec<syn::PathSegment> =
                        tp.path.segments.iter().skip(q.position).cloned().collect();
                    if segs.is_empty() {
                        return Ok(base);
                    }
                    return resolve_self_projection(&segs, ty.span(), ctx, depth);
                }
                if mentions_self(&base) {
                    return Err(syn::Error::new(
                        ty.span(),
                        "#[decycle(structural)]: cannot resolve a qualified projection whose base \
                         mentions `Self` without being `Self` (e.g. `<Box<Self> as Tr>::Out`). \
                         Spell the concrete type instead.",
                    ));
                }
                // Self-free base: keep the projection, normalizing base + segment arguments.
                let mut tp2 = tp.clone();
                tp2.qself.as_mut().unwrap().ty = Box::new(base);
                for seg in tp2.path.segments.iter_mut() {
                    normalize_in_args(&mut seg.arguments, ctx, depth)?;
                }
                return Ok(Type::Path(tp2));
            }
            let mut tp2 = tp.clone();
            for seg in tp2.path.segments.iter_mut() {
                normalize_in_args(&mut seg.arguments, ctx, depth)?;
            }
            Ok(Type::Path(tp2))
        }
        Type::Reference(r) => {
            let mut r = r.clone();
            r.elem = Box::new(normalize_projections(&r.elem, ctx, depth)?);
            Ok(Type::Reference(r))
        }
        Type::Ptr(p) => {
            let mut p = p.clone();
            p.elem = Box::new(normalize_projections(&p.elem, ctx, depth)?);
            Ok(Type::Ptr(p))
        }
        Type::Array(a) => {
            let mut a = a.clone();
            a.elem = Box::new(normalize_projections(&a.elem, ctx, depth)?);
            Ok(Type::Array(a))
        }
        Type::Slice(s) => {
            let mut s = s.clone();
            s.elem = Box::new(normalize_projections(&s.elem, ctx, depth)?);
            Ok(Type::Slice(s))
        }
        Type::Paren(p) => {
            let mut p = p.clone();
            p.elem = Box::new(normalize_projections(&p.elem, ctx, depth)?);
            Ok(Type::Paren(p))
        }
        Type::Group(g) => {
            let mut g = g.clone();
            g.elem = Box::new(normalize_projections(&g.elem, ctx, depth)?);
            Ok(Type::Group(g))
        }
        Type::Tuple(t) => {
            let mut t2 = t.clone();
            t2.elems.clear();
            for e in &t.elems {
                t2.elems.push(normalize_projections(e, ctx, depth)?);
            }
            Ok(Type::Tuple(t2))
        }
        Type::BareFn(bf) => {
            let mut bf = bf.clone();
            for input in bf.inputs.iter_mut() {
                input.ty = normalize_projections(&input.ty, ctx, depth)?;
            }
            if let syn::ReturnType::Type(_, t) = &mut bf.output {
                *t = Box::new(normalize_projections(t, ctx, depth)?);
            }
            Ok(Type::BareFn(bf))
        }
        Type::TraitObject(to) => {
            let mut to = to.clone();
            normalize_in_bounds(&mut to.bounds, ctx, depth)?;
            Ok(Type::TraitObject(to))
        }
        Type::ImplTrait(it) => {
            let mut it = it.clone();
            normalize_in_bounds(&mut it.bounds, ctx, depth)?;
            Ok(Type::ImplTrait(it))
        }
        other => Ok(other.clone()),
    }
}

fn normalize_in_args(args: &mut PathArguments, ctx: &ProjCtx, depth: u8) -> syn::Result<()> {
    match args {
        PathArguments::AngleBracketed(ab) => {
            for a in ab.args.iter_mut() {
                match a {
                    GenericArgument::Type(t) => *t = normalize_projections(t, ctx, depth)?,
                    GenericArgument::AssocType(at) => {
                        at.ty = normalize_projections(&at.ty, ctx, depth)?;
                    }
                    _ => {}
                }
            }
        }
        PathArguments::Parenthesized(p) => {
            for t in p.inputs.iter_mut() {
                *t = normalize_projections(t, ctx, depth)?;
            }
            if let syn::ReturnType::Type(_, t) = &mut p.output {
                *t = Box::new(normalize_projections(t, ctx, depth)?);
            }
        }
        PathArguments::None => {}
    }
    Ok(())
}

fn normalize_in_bounds(
    bounds: &mut syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    ctx: &ProjCtx,
    depth: u8,
) -> syn::Result<()> {
    for b in bounds.iter_mut() {
        if let syn::TypeParamBound::Trait(tb) = b {
            for seg in tb.path.segments.iter_mut() {
                normalize_in_args(&mut seg.arguments, ctx, depth)?;
            }
        }
    }
    Ok(())
}

/// Resolve the projection chain `Self::segs[0]::segs[1]::..` — `segs[0]` must be an associated type
/// defined in THIS impl; its definition becomes the new base, and any remaining segments project off
/// that (recursing while the base keeps resolving to `Self`).
fn resolve_self_projection(
    segs: &[syn::PathSegment],
    span: Span,
    ctx: &ProjCtx,
    depth: u8,
) -> syn::Result<Type> {
    let first = &segs[0];
    let name = first.ident.to_string();
    let Some((def, has_generics)) = ctx.assoc.get(&name) else {
        return Err(syn::Error::new(
            span,
            format!(
                "#[decycle(structural)]: cannot resolve the projection `Self::{name}` — `{name}` \
                 is not an associated type defined in this impl (a supertrait's or another trait's \
                 associated type?), so the engine cannot tell whether it names the cycle type. \
                 Spell the concrete type instead.",
            ),
        ));
    };
    if *has_generics || !matches!(first.arguments, PathArguments::None) {
        return Err(syn::Error::new(
            span,
            format!(
                "#[decycle(structural)]: generic associated type projections (`Self::{name}<..>`) \
                 are not supported. Spell the concrete type instead.",
            ),
        ));
    }
    let base = normalize_projections(def, ctx, depth + 1)?;
    if segs.len() == 1 {
        return Ok(base);
    }
    // A chained projection `Self::Out::Inner`: keep resolving while the base is `Self` again …
    if is_plain_self(&base) {
        return resolve_self_projection(&segs[1..], span, ctx, depth + 1);
    }
    if mentions_self(&base) {
        return Err(syn::Error::new(
            span,
            "#[decycle(structural)]: cannot resolve a chained `Self` projection whose intermediate \
             type mentions `Self` without being `Self`. Spell the concrete type instead.",
        ));
    }
    // … otherwise the rest projects off a Self-free base: emit `<base>::Rest` and let rustc resolve
    // it (nothing left for the cast to care about).
    let path = syn::Path {
        leading_colon: None,
        segments: segs[1..].iter().cloned().collect(),
    };
    Ok(Type::Path(syn::TypePath {
        qself: Some(syn::QSelf {
            lt_token: Default::default(),
            ty: Box::new(base),
            position: 0,
            as_token: None,
            gt_token: Default::default(),
        }),
        path,
    }))
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
    let needs = Ident::new(
        "stripped_wrapped_bound_needs_its_container_to_forward_the_trait",
        Span::call_site(),
    );
    let assert = Ident::new("__decycle_check_wrapped_bound_forwards", Span::call_site());
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

/// Does `path` mention any of `idents` — as a segment, a type argument, **or inside an
/// associated-type binding**?
///
/// The binding case is not a detail: a cyclic bound of the form `Tr<Assoc = X>` puts `X` somewhere no
/// type *argument* appears, and `X` is very often an invented impl generic (syan's `Spanned` cyclic
/// bound always carries `Span = __Syan_Span`). Missing it made the forwarding-assertion guard above
/// fail to fire, and the assertion was then emitted referring to a parameter that is not in scope
/// inside its `const _` block — `E0412: cannot find type __Syan_Span`, pointing at the macro rather
/// than at anything the caller wrote.
fn path_mentions(path: &syn::Path, idents: &HashSet<String>) -> bool {
    let mut found = false;
    for seg in &path.segments {
        if idents.contains(&seg.ident.to_string()) {
            found = true;
        }
        if let PathArguments::AngleBracketed(ab) = &seg.arguments {
            for a in &ab.args {
                let mentions = match a {
                    GenericArgument::Type(t) => type_mentions(t, idents),
                    GenericArgument::AssocType(at) => type_mentions(&at.ty, idents),
                    // A `Tr<Assoc: Bound>` constraint can name a param in the bound itself.
                    GenericArgument::Constraint(c) => c.bounds.iter().any(|b| match b {
                        syn::TypeParamBound::Trait(tb) => path_mentions(&tb.path, idents),
                        _ => false,
                    }),
                    _ => false,
                };
                if mentions {
                    found = true;
                }
            }
        }
    }
    found
}

// Generics rendering (`params_decl`/`params_use`/`wrap_angle`) lives in `crate::generics_fmt`,
// shared with the ranked engine.
