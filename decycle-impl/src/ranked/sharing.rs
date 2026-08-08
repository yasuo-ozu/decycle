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
//!
//! **Scope.** An impl only ever *needs* a sibling's premises when its own rank chain can reach that
//! sibling's delegating impl — that is, when its (transitive) cyclic bounds obligate it. The caller
//! therefore says, per impl, which other impls may contribute (`sources`), computed from the actual
//! obligation edges. An impl of the same trait that shares no obligation path — a standalone acyclic
//! impl, or a member of a *different*, disjoint cycle — receives nothing, so it does not silently
//! acquire premises (`C<T>` acquiring `T: Clone`) that reject code valid without the macro.

use proc_macro2::Ident;
use std::collections::{HashMap, HashSet};
use syn::visit_mut::VisitMut;
use syn::{ItemImpl, TypeParamBound, WherePredicate};
use template_quote::quote;

/// Apply the union to one trait's impls — see the module docs.
///
/// A cycle member's body reaches its siblings through the *public* trait (`Vec<Expr<S>>: Unparse<A>`
/// ⇒ `Expr<S>: Unparse<A>`), which decycle discharges through `Expr`'s delegating impl — and that
/// impl carries `Expr`'s **own** leaf bounds. A sibling (or a `#[group]` substruct) whose leaf set is
/// smaller therefore cannot prove it from its own `where`-clause. Sharing the union closes that gap;
/// it is the same idea as the `#[predicate_unparse(<leaf union>)]` injection the direct
/// `Unparse`/`Spanned` path uses, computed from the real predicates rather than re-derived from field
/// types (so a `#[group]`'s `Fill` obligation is covered too).
///
/// `sources[i]` lists the indices (into `impls`) of the impls whose premises `impls[i]` may inherit —
/// the impls its rank chain can actually reach, as computed by the caller from the obligation edges.
/// `routed_traits` is the full set of `#[decycle]`-routed trait idents: a predicate whose bound names
/// any of them in the bare (or `self::`-qualified) spelling is a cycle edge for the engine to rank,
/// never a premise to copy — copying one onto a sibling would rank-lower it there too, and a
/// cross-trait edge (`X: Tr2` inside an impl of `Tr`) is stripped downstream exactly like an own-trait
/// one (`remove_cyclic_bounds` keys on every routed trait), so both spellings are excluded alike.
///
/// A predicate is only injected into an impl that declares every generic parameter it mentions, so a
/// cycle whose members carry *different* parameters stays well-formed.
pub(crate) fn share_side_predicates_scoped(
    impls: &mut [&mut ItemImpl],
    routed_traits: &HashSet<Ident>,
    sources: &[Vec<usize>],
) {
    debug_assert_eq!(impls.len(), sources.len());
    // Per-impl data up front — the injection loop below holds the mutable borrow.
    let param_sets: Vec<HashSet<String>> = impls.iter().map(|im| impl_param_idents(im)).collect();
    let shareable: Vec<Vec<WherePredicate>> = impls
        .iter()
        .map(|im| {
            im.generics
                .where_clause
                .iter()
                .flat_map(|wc| wc.predicates.iter())
                // Skip the cyclic bounds (spelled bare or `self::`-qualified, naming any routed
                // trait): those are decycle's to rank, and copying one onto a sibling would
                // rank-lower it there too.
                .filter(|p| !is_bare_trait_pred(p, routed_traits))
                // `Self` is impl-relative: `Self: Debug` on `impl Tr for A` says *A* is `Debug`, so
                // copying it onto `impl Tr for B` silently asserts something about a different type
                // (and fails if B is not `Debug`). Only impl-independent premises can be shared.
                .filter(|p| !mentions_self(p))
                .cloned()
                .collect()
        })
        .collect();
    for (i, im) in impls.iter_mut().enumerate() {
        // The union this impl may inherit, from its sources only.
        let mut union: Vec<WherePredicate> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for j in sources[i].iter() {
            for p in &shareable[*j] {
                if seen.insert(quote!(#p).to_string()) {
                    union.push(p.clone());
                }
            }
        }
        if union.is_empty() {
            continue;
        }
        // Idents that are a generic parameter of this impl or of one of its sources — used below to
        // tell a parameter apart from a concrete type name. Scoped to the receiver's own slice of the
        // group on purpose: it was previously a `thread_local!` that accumulated and was never
        // cleared, so parameters leaked between separate `#[decycle]` expansions compiled on the same
        // thread, making unrelated idents look param-like and silently suppressing predicate sharing.
        let group_params: HashSet<String> = sources[i]
            .iter()
            .flat_map(|j| param_sets[*j].iter())
            .chain(param_sets[i].iter())
            .cloned()
            .collect();
        let params = &param_sets[i];
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

/// [`share_side_predicates_scoped`] with the whole group as every impl's source — each member both
/// contributes to and receives the full union.
///
/// Kept for the structural engine's graph path (`structural::expand`), whose group is the
/// caller-GENERATED impls of one trait under a supplied graph: there the caller has already stated
/// the participant set, and the generated helper impls are siblings of one cycle by construction.
/// The ranked engine instead scopes each impl's sources by obligation reachability — see
/// [`crate::ranked::process_module`]'s `sharing_sources`.
pub(crate) fn share_side_predicates(impls: &mut [&mut ItemImpl], cyclic_marker: &Ident) {
    let routed: HashSet<Ident> = core::iter::once(cyclic_marker.clone()).collect();
    let everyone: Vec<Vec<usize>> = (0..impls.len())
        .map(|i| (0..impls.len()).filter(|j| *j != i).collect())
        .collect();
    share_side_predicates_scoped(impls, &routed, &everyone);
}

/// The generic-parameter idents an impl declares — used to decide whether a sibling's predicate can be
/// injected into it (see [`share_side_predicates_scoped`]).
fn impl_param_idents(item_impl: &ItemImpl) -> HashSet<String> {
    item_impl
        .generics
        .params
        .iter()
        .map(|p| match p {
            syn::GenericParam::Type(t) => t.ident.to_string(),
            syn::GenericParam::Const(c) => c.ident.to_string(),
            // Lifetimes count too. Leaving them out let a predicate naming a SIBLING's lifetime
            // (`where &'a str: Clone`) past the "declares everything it mentions" guard and into
            // an impl with no `'a` of its own — E0261 on code that is valid without the macro.
            // `freshen_binder_lifetimes` does not help: it renames `for<>` binders, not free
            // lifetimes.
            syn::GenericParam::Lifetime(l) => l.lifetime.ident.to_string(),
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
        // `'static` and `'_` are always in scope, so they must not count as something the
        // receiving impl has to declare — otherwise, now that `impl_param_idents` reports
        // lifetimes, an ordinary `T: 'static` co-bound would never be shared. Deliberately does
        // not recurse: the default impl would feed the bare ident to `visit_ident`.
        fn visit_lifetime(&mut self, lt: &syn::Lifetime) {
            if lt.ident != "static" && lt.ident != "_" {
                self.0.insert(lt.ident.to_string());
            }
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

/// Does the predicate carry a bound the engine treats as a cycle edge — a bare (or
/// `self::`-qualified, which peel/validate normalise the same way) reference to **any** routed
/// trait? Matching only the group's own trait missed two spellings of the same edge: `B: self::Tr`
/// (identical to `B: Tr` everywhere else in the engine) and a cross-trait `X: Tr2`, which
/// `remove_cyclic_bounds` strips and rank-lowers exactly like an own-trait bound.
fn is_bare_trait_pred(pred: &WherePredicate, routed_traits: &HashSet<Ident>) -> bool {
    let WherePredicate::Type(pt) = pred else {
        return false;
    };
    pt.bounds.iter().any(|b| match b {
        TypeParamBound::Trait(tb) => {
            let mut path = tb.path.clone();
            crate::helper::strip_leading_self(&mut path);
            path.segments.len() == 1 && routed_traits.contains(&path.segments[0].ident)
        }
        _ => false,
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
