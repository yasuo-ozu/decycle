//! The **structural unroll** algorithm (`#[decycle(structural)]`): break infinite trait-obligation
//! cycles by giving each cyclic type a `#[repr(transparent)]` terminator `__XxxTerm` that carries the
//! trait impl (body via the trait-def-inside-body pattern) and layout-casting the natural type's
//! delegating impl to it. Emits no runtime machinery — a simpler sibling of the `ranked` engine.
//!
//! Unlike the ranked engine it does NOT scan every impl: only impls of the traits annotated
//! `#[decycle]` in the module participate (`collect_decycle_traits`), matching the ranked convention.
//!
//! Scope: trait methods with no growing type arguments. A method whose generic argument grows per
//! level (the `Dup`/stream-tower case) is not a scope limit of *this* engine — no engine can break it,
//! since the growth is in the monomorphisation rather than the obligation — and `crate::growth` now
//! rejects it up-front for both. Handles associated fns + every `self`-receiver shape, method generics,
//! APIT, associated items, multiple traits per cycle, and multiroot SCCs; emits a forwarding assertion
//! when a wrapped container predicate (`Box<Stmt>: Tr`) is stripped.
//!
//! Method generics are supported *except* when a bound mentions `Self` (`fn apply<F: Fn(&Self)>`):
//! this engine emits the method twice, once with `Self` = the terminator and once with `Self` = the
//! natural type, and one caller-supplied `F` cannot satisfy both. Such a bound is rejected with an
//! explanation pointing at `fn(&Self)` / `&dyn Fn(&Self)`, which the layout cast CAN carry across.
//! An impl is free to spell any `Self` position with the concrete self type (`&A` for a declared
//! `&Self`); the signature is canonicalized against the trait's declaration before codegen.

use proc_macro2::{Span, TokenStream};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use syn::{
    parse_quote, spanned::Spanned, Attribute, GenericArgument, GenericParam, Ident, ImplItem, Item,
    ItemEnum, ItemImpl, ItemMod, ItemStruct, Path, PathArguments, ReturnType, Type, WherePredicate,
};
use template_quote::{quote, ToTokens};

mod analyze;
mod codegen;
mod collect;
mod rho;
#[cfg(test)]
mod tests;

// The bare-name rule a participant is read by, shared with the ranked engine.
use crate::names::bare_ident;

pub(crate) use analyze::*;
pub(crate) use codegen::*;
pub(crate) use collect::*;
pub(crate) use rho::*;

/// Apply the structural `#[decycle(structural)]` transformation to `module`, programmatically —
/// the no-runtime unroll. `decycle` is the path to the decycle crate (its leading segment names the
/// crate, used to recognise `#[<crate>::decycle]` on inner items). For macro authors wrapping
/// `#[decycle(structural)]`; most users should use the attribute.
pub fn process_module(module: ItemMod, decycle: &Path) -> TokenStream {
    expand_or_fall_back(module, decycle, None)
}

/// [`process_module`] for a module whose absolute path is `module_path` — the programmatic form of
/// `#[decycle(structural, module = path)]`.
///
/// `module_path` must be absolute (`crate::..`, `$crate::..` or `::..`), without generic arguments,
/// and end in the module's own name; anything else aborts (through `proc_macro_error`). The cycle is
/// solved on the module as written, where only a bare (or `self::`) name is the module's; once it
/// is, the items re-emitted as written (everything but the impls the unroll replaces) have each bare
/// or `self::` name of a type or trait declared in the module spelled `<module_path>::X`, so an
/// `#[announce]` on one of them sees absolute paths only. A check that `module_path` names this
/// module is emitted into it (an error at the path otherwise).
pub fn process_module_at(module: ItemMod, module_path: &Path, decycle: &Path) -> TokenStream {
    let scope = crate::module_path::ModuleScope::new(module_path, &module);
    expand_or_fall_back(module, decycle, Some(&scope))
}

fn expand_or_fall_back(
    module: ItemMod,
    decycle: &Path,
    scope: Option<&crate::module_path::ModuleScope>,
) -> TokenStream {
    match expand(module.clone(), decycle, scope) {
        Ok(ts) => ts,
        Err(e) => {
            // Re-emit the module verbatim alongside the error so downstream code still sees the defs.
            let err = e.to_compile_error();
            let module = as_written(module, scope);
            quote! { #err #module }
        }
    }
}

/// `module` re-emitted as written: qualified, and with the check of its path, under
/// `module = path`.
fn as_written(mut module: ItemMod, scope: Option<&crate::module_path::ModuleScope>) -> TokenStream {
    if let Some(scope) = scope {
        scope.qualify_module(&mut module);
        if let Some((_, items)) = &mut module.content {
            items.push(Item::Verbatim(scope.path_check()));
        }
    }
    module.to_token_stream()
}

/// `scope` is the `module = path` of the attribute, when given: the cycle is solved on the module as
/// written, and every item re-emitted as written is qualified on the way out, after it. See
/// `crate::module_path`.
fn expand(
    mut module: ItemMod,
    decycle: &Path,
    scope: Option<&crate::module_path::ModuleScope>,
) -> syn::Result<TokenStream> {
    let qualify = |item: &Item| match scope {
        Some(scope) => scope.qualify_item(item),
        None => item.clone(),
    };
    let decycle_crate = decycle
        .segments
        .first()
        .map(|s| s.ident.clone())
        .ok_or_else(|| syn::Error::new(decycle.span(), "empty decycle path"))?;

    // Before anything else: reject value-level instantiation growth (a cyclic method reborrowing its
    // own by-value generic parameter). Neither engine can break it, and left alone it surfaces as a
    // monomorphisation limit naming generated internals. See `crate::growth`.
    crate::growth::check_value_generic_growth(&module, decycle)?;

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
    // and strip the `#[decycle]` attrs from the trait/use items. A marked trait alias or glob
    // `use`, and a `#[decycle]` in a nested module, are rejected as the ranked engine rejects them.
    let decycle_traits = collect_decycle_traits(brace_items, &decycle_crate);
    if brace_items.iter().any(|item| matches!(item, Item::Mod(_))) {
        // On that abort the module is emitted as the ranked engine leaves it: its own markers
        // taken, a nested module as written (which then expands on its own), not the caller's
        // fallback.
        proc_macro_error::set_dummy(as_written(module.clone(), scope));
        for item in module.content.iter().flat_map(|(_, items)| items) {
            if let Item::Mod(nested) = item {
                crate::markers::check_submodule(nested, &decycle_crate);
            }
        }
    }
    let brace_items = &mut module
        .content
        .as_mut()
        .expect("an inline module, checked above")
        .1;

    let mut items = brace_items.clone();

    // No `#[decycle]`-annotated trait/use → no cyclic participants. Re-emit the module unchanged (its
    // inner `#[decycle]` attrs were already stripped in place): the impls are valid Rust on their own,
    // so a `structural` annotation on a module with nothing to unroll is a silent no-op, not an error.
    // (The ranked engine instead REQUIRES an annotated cycle and rejects this — `NO_DECYCLE_TRAITS_MSG`.)
    if decycle_traits.is_empty() {
        return Ok(as_written(module, scope));
    }

    let mut model = Model::collect(&items, nonce)?;
    let sccs = model.cyclic_sccs(&decycle_traits);
    // Annotated traits present but they form no cycle — the same silent no-op pass-through.
    if sccs.is_empty() {
        return Ok(as_written(module, scope));
    }

    // The impls of one cycle share their non-cyclic premises. A member's generated terminator impl
    // reaches the impls its cyclic bounds name through the public trait, which their natural impls
    // discharge — and those demand their OWN leaf premises, which the member's `where`-clause need
    // not state (a generated helper type's `Leaf: Parse<A>`, members written with different leaf
    // bounds, or the impl of another trait in a cycle that alternates between traits). Plain Rust
    // proves those through the cyclic bound itself; the unroll strips that bound, so the member needs
    // them in its own environment. Scoped to the members of the cycle: an impl outside every cycle,
    // and a disjoint cycle, acquire nothing. Adding them does not change what a natural impl requires
    // in effect — the stripped cyclic bound required them already, transitively, at the instantiation
    // the bound names, which is the one the premises are mapped to (see `share_cycle_premises`; the
    // by-name pass kept beside it can over-state that, where two impls of one trait name different
    // things alike).
    if share_cycle_premises(brace_items, &sccs, &model) {
        items = brace_items.clone();
        model = Model::collect(&items, nonce)?;
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
                out.extend(qualify(it).to_token_stream());
            }
            other => out.extend(qualify(other).to_token_stream()),
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

    // `module = path`: checked against the module, in the module (`ModuleScope::path_check`).
    if let Some(scope) = scope {
        out.extend(scope.path_check());
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

/// Give the impls of each cycle (`sccs`) the premises of the impls their cyclic bounds reach, in
/// place; whether any impl changed. See `expand`.
///
/// A premise is one of the `where`-predicates (or inline parameter bounds) `reduce_generics` keeps on
/// the generated impls: one carrying no bound that is an edge of the same cycle (a bound `X: Tr`
/// where `X` names a member `M` with `(M, Tr)` in the SCC). Both passes of [`crate::premise_flow`],
/// scoped to the members of one cycle:
///
/// - **along the bounds, across traits**: an impl receives the premises of every impl its cyclic
///   bounds reach, transitively and whatever their trait, with the reached impl's generic
///   parameters mapped through the bound (`Term: Size<T>` reaching
///   `impl<U> Size<U> for Term where U: Clone` gives `T: Clone`). A premise with no counterpart (a
///   parameter the bound does not fix) is skipped. Computed on the impls as written, before the
///   second pass adds anything.
/// - **by name, per trait** (as before): the impls of one trait in the cycle take the union of each
///   other's premises: a predicate is only injected into an impl that declares every generic
///   parameter it names, a `Self`-mentioning one is never shared, and an HRTB binder is renamed
///   apart from the receiver's lifetimes. It adds nothing the first pass gives when the parameters
///   correspond by name, and it still covers a bound the first pass cannot match.
///
/// The cycles have no impl in common, so each one is shared on its own.
fn share_cycle_premises(items: &mut [Item], sccs: &[Scc], model: &Model) -> bool {
    let adt_names: HashSet<String> = model.adts.keys().cloned().collect();
    let in_scc = |im: &ItemImpl, scc: &Scc| {
        impl_trait_key(im)
            .zip(impl_self_ident(im))
            .is_some_and(|(key, ty)| scc.contains(&ty.to_string(), &key))
    };
    let mut changed = false;
    for scc in sccs {
        let mut impls: Vec<&mut ItemImpl> = items
            .iter_mut()
            .filter_map(|item| match item {
                Item::Impl(im) if in_scc(im, scc) => Some(im),
                _ => None,
            })
            .collect();
        let edges: Vec<Vec<crate::premise_flow::CycleEdge>> = {
            let written: Vec<&ItemImpl> = impls.iter().map(|im| &**im).collect();
            written
                .iter()
                .map(|im| cycle_edges(im, &written, scc, &adt_names))
                .collect()
        };
        let mut traits: Vec<&String> = scc.pairs.iter().map(|(_, tr)| tr).collect();
        traits.sort();
        traits.dedup();
        let by_name: Vec<(Vec<usize>, Vec<Vec<usize>>)> = traits
            .into_iter()
            .filter_map(|trait_name| {
                let members: Vec<usize> = (0..impls.len())
                    .filter(|k| impl_trait_key(impls[*k]).as_deref() == Some(trait_name.as_str()))
                    .collect();
                let everyone = (0..members.len())
                    .map(|i| (0..members.len()).filter(|j| *j != i).collect())
                    .collect();
                (members.len() >= 2).then_some((members, everyone))
            })
            .collect();
        let is_edge =
            |im: &ItemImpl, pred: &WherePredicate| is_cycle_edge(im, pred, scc, &adt_names);
        changed |= crate::premise_flow::share_premises(
            &mut impls,
            &edges,
            &is_edge,
            crate::premise_flow::Respell::Any,
            &by_name,
        );
    }
    changed
}

/// The bounds of `im` that are edges of `scc`, for [`crate::premise_flow`]: one per member a bound
/// names (`Box<Term<S>>: Size<T>` is the edge to `Term<S>`), with the impls of `impls` (the cycle's)
/// that may discharge it.
fn cycle_edges(
    im: &ItemImpl,
    impls: &[&ItemImpl],
    scc: &Scc,
    adt_names: &HashSet<String>,
) -> Vec<crate::premise_flow::CycleEdge> {
    let mut out = Vec::new();
    for pt in crate::names::type_predicates(&im.generics) {
        let bounded = subst_self(&pt.bounded_ty, &im.self_ty);
        let members = local_ref_paths(&bounded, adt_names);
        for bound in &pt.bounds {
            let syn::TypeParamBound::Trait(tb) = bound else {
                continue;
            };
            let Some(key) = trait_key(&tb.path) else {
                continue;
            };
            for member in &members {
                let Some(name) = bare_ident(&member.path).map(|i| i.to_string()) else {
                    continue;
                };
                if scc.contains(&name, &key) {
                    out.push(crate::premise_flow::CycleEdge::new(
                        Type::Path(member.clone()),
                        pt,
                        tb,
                        impls,
                    ));
                }
            }
        }
    }
    out
}

/// Does `pred` (on `im`) carry a bound that is an edge of `scc` — the bounds `reduce_generics`
/// strips from the generated impls?
fn is_cycle_edge(
    im: &ItemImpl,
    pred: &WherePredicate,
    scc: &Scc,
    adt_names: &HashSet<String>,
) -> bool {
    let WherePredicate::Type(pt) = pred else {
        return false;
    };
    let refs = local_refs(&subst_self(&pt.bounded_ty, &im.self_ty), adt_names);
    pt.bounds.iter().any(|b| {
        matches!(b, syn::TypeParamBound::Trait(tb) if trait_key(&tb.path)
            .is_some_and(|key| refs.iter().any(|r| scc.contains(r, &key))))
    })
}

/// Collect the idents of traits annotated `#[decycle]` (either a `#[decycle] trait Foo` or a
/// `#[decycle] use path::Foo`) and strip the `#[decycle]` attributes from those items
/// ([`crate::markers::take_marked`], which rejects a marked trait alias or glob `use`).
fn collect_decycle_traits(items: &mut [Item], decycle_crate: &Ident) -> HashSet<String> {
    let mut traits = HashSet::new();
    for marked in crate::markers::take_marked(items, decycle_crate) {
        match marked {
            crate::markers::Marked::Trait(t) => {
                traits.insert(t.ident.to_string());
            }
            crate::markers::Marked::Use(u, names) => {
                traits.extend(names.iter().map(|i| i.to_string()));
                // This engine reads no trait through the `use`: its name is all it takes, and the
                // module's impls spell the trait by it. Nothing else may name the import (a
                // stand-in that is only the real trait's copy, reached under a rename), so it
                // would be reported unused.
                u.attrs.push(parse_quote!(#[allow(unused_imports)]));
            }
        }
    }
    traits
}
