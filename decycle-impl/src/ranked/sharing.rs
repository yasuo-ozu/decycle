//! Give every contracted impl of one cycle the **union** of its siblings' non-cyclic premises.
//!
//! A cycle member's body reaches its siblings through the *public* trait, and the engine discharges
//! that through the sibling's own delegating impl — which carries the sibling's **own** leaf bounds.
//! A member whose leaf set is smaller therefore cannot prove, from its own `where`-clause, what a
//! sibling's delegating impl demands. Sharing the union across one cycle's impls closes that gap.
//!
//! The union is computed from the impls' *real* predicates rather than re-derived from field types,
//! so an obligation a caller introduced by hand (or through a generated helper type) is covered too.
//! Peeled cyclic bounds are excluded: those are the engine's to rank, and copying one onto a sibling
//! would rank-lower it there as well.

use proc_macro2::Ident;
use std::collections::{HashMap, HashSet};
use syn::visit_mut::VisitMut;
use syn::{ItemImpl, TypeParamBound, WherePredicate};
use template_quote::quote;

/// Apply the union to one cycle group — see the module docs.
///
/// A cycle member's body reaches its siblings through the *public* trait (`Vec<Expr<S>>: Unparse<A>`
/// ⇒ `Expr<S>: Unparse<A>`), which decycle discharges through `Expr`'s delegating impl — and that
/// impl carries `Expr`'s **own** leaf bounds. A sibling (or a `#[group]` substruct) whose leaf set is
/// smaller therefore cannot prove it from its own `where`-clause. Sharing the union closes that gap;
/// it is the same idea as the `#[predicate_unparse(<leaf union>)]` injection the direct
/// `Unparse`/`Spanned` path uses, computed from the real predicates rather than re-derived from field
/// types (so a `#[group]`'s `Fill` obligation is covered too).
///
/// A predicate is only injected into an impl that declares every generic parameter it mentions, so a
/// cycle whose members carry *different* parameters stays well-formed.
pub(crate) fn share_side_predicates(impls: &mut [&mut ItemImpl], cyclic_marker: &Ident) {
    // Idents that are a generic parameter of *some* impl in THIS group — used below to tell a
    // parameter apart from a concrete type name. Scoped to the group on purpose: it was previously a
    // `thread_local!` that accumulated and was never cleared, so parameters leaked between separate
    // `#[decycle]` expansions compiled on the same thread, making unrelated idents look param-like
    // and silently suppressing predicate sharing.
    let group_params: HashSet<String> = impls.iter().flat_map(|im| impl_param_idents(im)).collect();
    let mut union: Vec<WherePredicate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for im in impls.iter() {
        let Some(wc) = &im.generics.where_clause else {
            continue;
        };
        for p in &wc.predicates {
            // Skip the peeled cyclic bounds (spelled with the bare trait ident): those are decycle's
            // to rank, and copying one onto a sibling would rank-lower it there too.
            if is_bare_trait_pred(p, cyclic_marker) {
                continue;
            }
            // `Self` is impl-relative: `Self: Debug` on `impl Tr for A` says *A* is `Debug`, so
            // copying it onto `impl Tr for B` silently asserts something about a different type
            // (and fails if B is not `Debug`). Only impl-independent premises can be shared.
            if mentions_self(p) {
                continue;
            }
            if seen.insert(quote!(#p).to_string()) {
                union.push(p.clone());
            }
        }
    }
    for im in impls.iter_mut() {
        let params = impl_param_idents(im);
        let lifetimes: HashSet<String> = im
            .generics
            .params
            .iter()
            .filter_map(|p| match p {
                syn::GenericParam::Lifetime(l) => Some(l.lifetime.ident.to_string()),
                _ => None,
            })
            .collect();
        let have: HashSet<String> = im
            .generics
            .where_clause
            .iter()
            .flat_map(|wc| wc.predicates.iter())
            .map(|p| quote!(#p).to_string())
            .collect();
        let mut add: Vec<WherePredicate> = Vec::new();
        for p in &union {
            if have.contains(&quote!(#p).to_string()) {
                continue;
            }
            // Every *declared* parameter the predicate names must exist here too. An ident that is
            // not a parameter anywhere (a concrete type, a trait) is not a constraint.
            let idents = pred_idents(p);
            let missing = idents
                .iter()
                .any(|i| !params.contains(i) && group_params.contains(i));
            if missing {
                continue;
            }
            // A shared HRTB predicate (`for<'a> <G as EmptyGroup>::Fill<Sub<'a, ..>>: …`) can carry a
            // binder lifetime that the receiving impl already declares as a parameter — a `#[group]`
            // substruct's impl declares exactly the lifetime its own predicate binds. Rename the
            // binder rather than shadowing it (E0496); an HRTB binder name is not observable.
            add.push(freshen_binder_lifetimes(p.clone(), &lifetimes));
        }
        if add.is_empty() {
            continue;
        }
        let wc = im
            .generics
            .where_clause
            .get_or_insert_with(|| syn::parse_quote!(where));
        for p in add {
            wc.predicates.push(p);
        }
    }
}

/// The type-parameter idents an impl declares — used to decide whether a sibling's predicate can be
/// injected into it (see [`share_side_predicates`]).
fn impl_param_idents(item_impl: &ItemImpl) -> HashSet<String> {
    item_impl
        .generics
        .params
        .iter()
        .filter_map(|p| match p {
            syn::GenericParam::Type(t) => Some(t.ident.to_string()),
            syn::GenericParam::Const(c) => Some(c.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// Every single-segment ident appearing anywhere in a predicate.
fn pred_idents(pred: &WherePredicate) -> HashSet<String> {
    struct V(HashSet<String>);
    impl syn::visit::Visit<'_> for V {
        fn visit_ident(&mut self, i: &Ident) {
            self.0.insert(i.to_string());
        }
    }
    let mut v = V(HashSet::new());
    syn::visit::Visit::visit_where_predicate(&mut v, pred);
    v.0
}

/// Rename a predicate's `for<'a, …>` binder lifetimes that collide with `taken`.
fn freshen_binder_lifetimes(mut pred: WherePredicate, taken: &HashSet<String>) -> WherePredicate {
    struct Ren<'a>(&'a HashMap<String, syn::Lifetime>);
    impl VisitMut for Ren<'_> {
        fn visit_lifetime_mut(&mut self, lt: &mut syn::Lifetime) {
            if let Some(new) = self.0.get(&lt.ident.to_string()) {
                *lt = new.clone();
            }
        }
    }
    let WherePredicate::Type(pt) = &mut pred else {
        return pred;
    };
    let mut rename: HashMap<String, syn::Lifetime> = HashMap::new();
    if let Some(bl) = &mut pt.lifetimes {
        for p in bl.lifetimes.iter_mut() {
            if let syn::GenericParam::Lifetime(l) = p {
                let old = l.lifetime.ident.to_string();
                if taken.contains(&old) {
                    let fresh =
                        syn::Lifetime::new(&format!("'{}__decycle_hr", old), l.lifetime.ident.span());
                    l.lifetime = fresh.clone();
                    rename.insert(old, fresh);
                }
            }
        }
    }
    if !rename.is_empty() {
        let mut r = Ren(&rename);
        if let WherePredicate::Type(pt) = &mut pred {
            r.visit_type_mut(&mut pt.bounded_ty);
            for b in pt.bounds.iter_mut() {
                r.visit_type_param_bound_mut(b);
            }
        }
    }
    pred
}

fn is_bare_trait_pred(pred: &WherePredicate, trait_ident: &Ident) -> bool {
    let WherePredicate::Type(pt) = pred else {
        return false;
    };
    pt.bounds.iter().any(|b| {
        matches!(b, TypeParamBound::Trait(tb)
            if tb.path.segments.len() == 1 && &tb.path.segments[0].ident == trait_ident)
    })
}

/// Does the predicate mention `Self` anywhere? Such a premise cannot be shared between impls.
fn mentions_self(pred: &WherePredicate) -> bool {
    struct V(bool);
    impl syn::visit::Visit<'_> for V {
        fn visit_ident(&mut self, i: &Ident) {
            if i == "Self" {
                self.0 = true;
            }
        }
    }
    let mut v = V(false);
    syn::visit::Visit::visit_where_predicate(&mut v, pred);
    v.0
}
