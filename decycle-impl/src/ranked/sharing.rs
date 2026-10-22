//! Give every impl of one cycle the **union** of its siblings' non-cyclic premises — both engines.
//!
//! A cycle member's body reaches its siblings through the *public* trait, and the engine discharges
//! that through the sibling's own delegating impl — which carries the sibling's **own** leaf bounds.
//! A member whose leaf set is smaller therefore cannot prove, from its own `where`-clause, what a
//! sibling's delegating impl demands. Sharing the union across one cycle's impls closes that gap.
//!
//! The union is computed from the impls' *real* predicates rather than re-derived from field types,
//! so an obligation a caller introduced by hand (or through a generated helper type) is covered too.
//! The cycle's own edges are excluded — which predicates those are is the engine's to say
//! (`is_edge`): the ranked engine ranks them, and copying one onto a sibling would rank-lower it
//! there as well; the structural engine strips them from the generated impls.
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
/// the impls it can actually reach, as computed by the caller from the obligation edges.
/// `is_edge(im, pred)` says whether a predicate of `im` is an edge of the cycle rather than a premise
/// — never shared (see [`is_ranked_edge`] and the structural engine's `is_cycle_edge`).
///
/// A predicate is only injected into an impl that declares every generic parameter it mentions, so a
/// cycle whose members carry *different* parameters stays well-formed.
pub(crate) fn share_side_predicates_scoped(
    impls: &mut [&mut ItemImpl],
    is_edge: &dyn Fn(&ItemImpl, &WherePredicate) -> bool,
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
                // Skip the cycle's edges: those are the engine's to rank or strip.
                .filter(|p| !is_edge(im, p))
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

/// The ranked engine's edges, for [`share_side_predicates_scoped`]: a predicate with a bound naming
/// a routed trait in the bare (or `self::`-qualified) spelling, on a target the rank chain descends.
///
/// `routed_traits` is the full set of `#[decycle]`-routed trait idents: a cross-trait edge (`X: Tr2`
/// inside an impl of `Tr`) is stripped downstream exactly like an own-trait one
/// (`remove_cyclic_bounds` keys on every routed trait). `participants` are the head idents of the
/// impls the engine takes over — the targets `remove_cyclic_bounds` reads a bare bound as an edge
/// on, besides `Self` and the impl's own type parameters. A bare bound on any other target
/// (`crate::other::Leaf: Tr`, `u8: Tr`, or the parts `peel` keeps of a wrapped bound) is no edge but
/// a premise on the original trait, which `finalize` keeps un-lowered, and it is shared like any
/// other premise. (Not sharing it left a sibling unable to prove what the rank chain reaching this
/// impl demands, so its re-entry fn was never registered, and a recursion past the floor panicked.)
pub(crate) fn is_ranked_edge(
    im: &ItemImpl,
    pred: &WherePredicate,
    routed_traits: &HashSet<Ident>,
    participants: &HashSet<Ident>,
) -> bool {
    is_bare_trait_pred(pred, routed_traits)
        && targets_participant(pred, participants, &impl_param_idents(im))
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

/// Is the predicate's bounded type one the ranked chain descends — `Self`, one of the impl's own type
/// parameters (`params`), or a type whose head is a participant? The same test as
/// `finalize::remove_cyclic_bounds`, which keys the head by its last segment.
fn targets_participant(
    pred: &WherePredicate,
    heads: &HashSet<Ident>,
    params: &HashSet<String>,
) -> bool {
    let WherePredicate::Type(pt) = pred else {
        return false;
    };
    if let syn::Type::Path(syn::TypePath { qself: None, path }) = &pt.bounded_ty {
        if path.is_ident("Self")
            || (path.segments.len() == 1 && params.contains(&path.segments[0].ident.to_string()))
        {
            return true;
        }
    }
    crate::helper::type_head_ident(&pt.bounded_ty).is_some_and(|h| heads.contains(&h))
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
