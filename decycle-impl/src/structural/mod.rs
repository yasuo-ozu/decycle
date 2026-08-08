//! The **structural unroll** algorithm (`#[decycle(structural)]`): break infinite trait-obligation
//! cycles by giving each cyclic type a `#[repr(transparent)]` terminator `__XxxTerm` that carries the
//! trait impl (body via the trait-def-inside-body pattern) and layout-casting the natural type's
//! delegating impl to it. Emits no runtime machinery — a simpler sibling of the `ranked` engine.
//!
//! Unlike the ranked engine it does NOT scan every impl: only impls of the traits annotated
//! `#[decycle]` in the module participate (`collect_decycle_traits`), matching the ranked convention.
//!
//! Scope: trait methods with no growing type arguments (the `Dup`/stream-tower case needs the ranked
//! engine). Handles associated fns + every `self`-receiver shape, method generics, APIT, associated
//! items, multiple traits per cycle, and multiroot SCCs; emits a forwarding assertion when a wrapped
//! container predicate (`Box<Stmt>: Tr`) is stripped.

use proc_macro2::{Span, TokenStream};
use template_quote::{quote, ToTokens};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use syn::{
    parse_quote, spanned::Spanned, Attribute, GenericArgument, GenericParam, Ident, ImplItem, Item,
    ItemEnum, ItemImpl, ItemMod, ItemStruct, Path, PathArguments, ReturnType, Type, UseTree,
    WherePredicate,
};

mod analyze;
mod codegen;
mod collect;
mod rho;

pub(crate) use analyze::*;
pub(crate) use codegen::*;
pub(crate) use collect::*;
pub(crate) use rho::*;

/// Apply the structural `#[decycle(structural)]` transformation to `module`, programmatically —
/// the no-runtime unroll. `decycle` is the path to the decycle crate (its leading segment names the
/// crate, used to recognise `#[<crate>::decycle]` on inner items). For macro authors wrapping
/// `#[decycle(structural)]`; most users should use the attribute.
pub fn process_module(module: ItemMod, decycle: &Path) -> TokenStream {
    match expand(module.clone(), None, decycle) {
        Ok(ts) => ts,
        Err(e) => {
            // Re-emit the module verbatim alongside the error so downstream code still sees the defs.
            let err = e.to_compile_error();
            quote! { #err #module }
        }
    }
}

/// [`process_module`], but with participation restricted to the types named by `graph`.
///
/// `graph` is an obligation graph in the shape [`crate::analysis::analyze_module`] returns.
///
/// **This engine's model is finer-grained than the graph.** It works over `(type, trait)` *pairs*,
/// because a type implementing two routed traits can be cyclic in one and acyclic in the other; the
/// graph collapses that, keying only on the type. So the node set is applied as a **filter**, not a
/// replacement: a pair participates when its trait is `#[decycle]`-annotated *and* its self type is a
/// node. Passing `analyze_module(&module, decycle)` therefore reproduces [`process_module`] only when
/// every participating type is a node, which is exactly what that function returns — so the round
/// trip is faithful, but a hand-built graph can only ever narrow the cycle, never widen it or split
/// it per trait.
///
/// The **edges are not consumed**; `Peeled` bounds are still recognised by their own syntax. See
/// [`crate::ranked::process_module_with_graph`], which has the same caveat for the same reason.
pub fn process_module_with_graph(
    module: ItemMod,
    graph: &crate::safegraph::VecGraph<Ident, crate::analysis::EdgeKind>,
    decycle: &Path,
) -> TokenStream {
    use crate::safegraph::graph::Graph;
    let allowed: HashSet<String> = graph.nodes().map(|n| n.to_string()).collect();
    match expand(module.clone(), Some(&allowed), decycle) {
        Ok(ts) => ts,
        Err(e) => {
            let err = e.to_compile_error();
            quote! { #err #module }
        }
    }
}

fn expand(
    mut module: ItemMod,
    allowed_types: Option<&HashSet<String>>,
    decycle: &Path,
) -> syn::Result<TokenStream> {
    let decycle_crate = decycle
        .segments
        .first()
        .map(|s| s.ident.clone())
        .ok_or_else(|| syn::Error::new(decycle.span(), "empty decycle path"))?;

    // Per-expansion hygiene nonce, derived from the (pre-mutation) module tokens. Every generated
    // identifier carries it so nothing the engine emits can collide with a user identifier.
    let nonce = make_nonce(&module.to_token_stream());

    let brace_items = match &mut module.content {
        Some((_, items)) => items,
        None => {
            return Err(syn::Error::new(
                module.span(),
                "#[decycle(structural)] must be applied to an inline module with a body",
            ))
        }
    };

    // Only impls of traits annotated `#[decycle]` in this module participate — collect their idents
    // and strip the `#[decycle]` attrs from the trait/use items.
    let decycle_traits = collect_decycle_traits(brace_items, &decycle_crate);

    // A caller supplying a graph is generating these impls, which means a cycle member and its helper
    // types are separate impls that each declare only their OWN leaf premises — and the generated
    // terminator for one has to prove what a sibling's impl demands. Share the union across each
    // trait's impls first, the same gap `ranked::sharing` closes on the other engine. Skipped without a
    // graph: hand-written impls state their own premises, and rewriting a caller's `where`-clause
    // uninvited is the sort of thing the graph is the opt-in for.
    if allowed_types.is_some() {
        for trait_name in &decycle_traits {
            let marker = Ident::new(trait_name, Span::call_site());
            let mut group: Vec<&mut ItemImpl> = brace_items
                .iter_mut()
                .filter_map(|item| match item {
                    Item::Impl(im)
                        if impl_trait_key(im).as_deref() == Some(trait_name.as_str()) =>
                    {
                        Some(im)
                    }
                    _ => None,
                })
                .collect();
            if group.len() > 1 {
                crate::ranked::sharing::share_side_predicates(&mut group, &marker);
            }
        }
    }

    let items = brace_items.clone();

    // No `#[decycle]`-annotated trait/use → no cyclic participants. Re-emit the module unchanged (its
    // inner `#[decycle]` attrs were already stripped in place): the impls are valid Rust on their own,
    // so a `structural` annotation on a module with nothing to unroll is a silent no-op, not an error.
    // (The ranked engine instead REQUIRES an annotated cycle and rejects this — `NO_DECYCLE_TRAITS_MSG`.)
    if decycle_traits.is_empty() {
        return Ok(module.to_token_stream());
    }

    let model = Model::collect(&items, nonce)?;
    let sccs = model.cyclic_sccs(&decycle_traits, allowed_types);
    // Annotated traits present but they form no cycle — the same silent no-op pass-through.
    if sccs.is_empty() {
        return Ok(module.to_token_stream());
    }

    let mut replaced_impls: HashSet<(String, String)> = HashSet::new();
    let mut cycle_adts: HashSet<String> = HashSet::new();
    for scc in &sccs {
        for (ty, tr) in &scc.pairs {
            replaced_impls.insert((ty.clone(), tr.clone()));
            cycle_adts.insert(ty.clone());
        }
    }

    let mut out = TokenStream::new();

    // The one layout-cast helper. `a` is placed in `ManuallyDrop` and never moved again, so the
    // bitwise `transmute_copy` result is the only owner: moving `a` into a `forget` *after* the copy
    // (the previous shape) retagged any `Box`/`&mut` inside it and invalidated the copy's tags —
    // Stacked-Borrows UB for every by-value `Self` shape containing a `Box`. Reading through a shared
    // `&ManuallyDrop<A>` performs no such retag, so the returned value's provenance stays valid.
    // Defense-in-depth: `transmute_copy` reads `size_of::<__B>()` bytes from a `&__A` without any
    // compile-time size check, so a (hypothetical future) codegen bug producing a size mismatch would
    // be silent UB. `#Guard::<A,B>::OK` is a post-monomorphization `assert!` (MSRV-safe — inline
    // `const{}` is 1.79) that turns any such mismatch into a compile error at zero runtime cost.
    let cast = cast_ident(nonce);
    let guard = sizeguard_ident(nonce);
    out.extend(quote! {
        #[allow(dead_code)]
        struct #guard<__A, __B>(::core::marker::PhantomData<(fn() -> __A, fn() -> __B)>);
        #[allow(dead_code)]
        impl<__A, __B> #guard<__A, __B> {
            const OK: () = ::core::assert!(
                ::core::mem::size_of::<__A>() == ::core::mem::size_of::<__B>(),
                "decycle internal error: layout cast between differently-sized types",
            );
        }
        #[inline]
        #[allow(dead_code)]
        unsafe fn #cast<__A, __B>(a: __A) -> __B {
            let () = #guard::<__A, __B>::OK;
            let a = ::core::mem::ManuallyDrop::new(a);
            ::core::mem::transmute_copy::<::core::mem::ManuallyDrop<__A>, __B>(&a)
        }
    });

    // Re-emit every item (the `__MTerm` is `#[repr(transparent)]`, so the natural types need no repr
    // change), skipping impls that are replaced by generated ones.
    for it in &items {
        match it {
            Item::Impl(im) => {
                if let (Some(key), Some(self_id)) = (impl_trait_key(im), impl_self_ident(im)) {
                    if replaced_impls.contains(&(self_id.to_string(), key)) {
                        continue;
                    }
                }
                out.extend(it.to_token_stream());
            }
            other => out.extend(other.to_token_stream()),
        }
    }

    // One terminator per unique cycle member (deduped across SCCs, so a type cyclic under several
    // traits gets a single `__MTerm`).
    let mut cycle_members: Vec<&String> = cycle_adts.iter().collect();
    cycle_members.sort();
    for m in cycle_members {
        let adt = model
            .adts
            .get(m)
            .ok_or_else(|| syn::Error::new(Span::call_site(), format!("unknown ADT {m}")))?;
        out.extend(make_term_item(adt, model.nonce));
    }

    // Generated impls, per cyclic SCC.
    for scc in &sccs {
        out.extend(codegen_scc(&model, scc)?);
    }

    let attrs = &module.attrs;
    let vis = &module.vis;
    let unsafety = &module.unsafety;
    let mod_token = &module.mod_token;
    let ident = &module.ident;
    Ok(quote! {
        #(#attrs)*
        #vis #unsafety #mod_token #ident {
            #out
        }
    })
}

/// Collect the idents of traits annotated `#[decycle]` (either a `#[decycle] trait Foo` or a
/// `#[decycle] use path::Foo`) and strip the `#[decycle]` attribute from those items.
fn collect_decycle_traits(items: &mut [Item], decycle_crate: &Ident) -> HashSet<String> {
    let mut traits = HashSet::new();
    for item in items.iter_mut() {
        match item {
            Item::Trait(t) => {
                if take_decycle_attr(&mut t.attrs, decycle_crate) {
                    traits.insert(t.ident.to_string());
                }
            }
            Item::Use(u) => {
                if take_decycle_attr(&mut u.attrs, decycle_crate) {
                    collect_use_trait_idents(&u.tree, &mut traits);
                }
            }
            _ => {}
        }
    }
    traits
}

fn take_decycle_attr(attrs: &mut Vec<Attribute>, decycle_crate: &Ident) -> bool {
    let before = attrs.len();
    attrs.retain(|a| !crate::is_decycle_attribute(a, decycle_crate));
    attrs.len() != before
}

fn collect_use_trait_idents(tree: &UseTree, out: &mut HashSet<String>) {
    match tree {
        UseTree::Path(p) => collect_use_trait_idents(&p.tree, out),
        UseTree::Name(n) => {
            out.insert(n.ident.to_string());
        }
        UseTree::Rename(r) => {
            out.insert(r.rename.to_string());
        }
        UseTree::Group(g) => {
            for t in &g.items {
                collect_use_trait_idents(t, out);
            }
        }
        UseTree::Glob(_) => {}
    }
}
