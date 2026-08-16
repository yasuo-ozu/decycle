//! Peel container wrappers off a cyclic where-bound so the ranked engine has a head it can descend.
//!
//! The ranked engine rewrites a cyclic bound `X: Tr` to `X: TrRanked<Rank>`, which resolves only when
//! `X`'s **head** type has a ranked impl in this module. A bound written on a wrapped type —
//! `Vec<Box<Stmt<S>>>: Tr`, the natural shape when the obligation is generated from a struct field —
//! has head `Vec`, which has no ranked impl, so the rewrite produces nothing rustc can solve.
//!
//! The information needed to fix it is already present, though: the cycle member is right there
//! inside the type arguments. `Vec<Box<Stmt<S>>>: Tr` and `Stmt<S>: Tr` are the same obligation as far
//! as the cycle is concerned — the container's own impl supplies the rest — so this pass rewrites the
//! former into the latter. Only then is `validate_impl_where_bounds` able to insist that every
//! surviving cyclic bound has a rankable head, because by that point the ones that *could* be made
//! rankable already are.
//!
//! Bounds that are not cyclic, and cyclic bounds that already name a rankable head (or target `Self`
//! or one of the impl's own type parameters), are left exactly as written.

use std::collections::HashSet;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{
    Ident, ItemImpl, Token, TraitBound, Type, TypeParamBound, TypePath, WherePredicate,
};
use template_quote::quote;

/// Rewrite every peelable cyclic bound of `item_impl` in place.
pub(crate) fn peel_cyclic_bounds(
    item_impl: &mut ItemImpl,
    all_traits: &HashSet<Ident>,
    cycle_self_heads: &HashSet<Ident>,
    is_local_target: &dyn Fn(&Type) -> bool,
) {
    let Some(where_clause) = &mut item_impl.generics.where_clause else {
        return;
    };
    let mut out: Punctuated<WherePredicate, Token![,]> = Punctuated::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |p: WherePredicate, out: &mut Punctuated<WherePredicate, Token![,]>| {
        if seen.insert(quote!(#p).to_string()) {
            out.push(p);
        }
    };

    for pred in where_clause.predicates.iter() {
        let WherePredicate::Type(pt) = pred else {
            push(pred.clone(), &mut out);
            continue;
        };
        // Already fine: `Self` / an impl type-param (the engine handles those directly), or a head
        // that is itself a cycle member — recognised by the BARE (or `self::`-qualified) spelling
        // only, exactly like `cycle_types_within` below. Keying on the last segment made
        // `crate::other::Stmt` pass for the local member `Stmt`.
        if is_local_target(&pt.bounded_ty)
            || crate::helper::local_type_head_ident(&pt.bounded_ty)
                .is_some_and(|h| cycle_self_heads.contains(&h))
        {
            push(pred.clone(), &mut out);
            continue;
        }
        let (mut cyclic, other): (Vec<TypeParamBound>, Vec<TypeParamBound>) = pt
            .bounds
            .iter()
            .cloned()
            .partition(|b| is_bare_cyclic_bound(b, all_traits));
        // `self::Tr` and `Tr` mean the same edge; emit the canonical bare spelling.
        for b in cyclic.iter_mut() {
            if let TypeParamBound::Trait(TraitBound { path, .. }) = b {
                crate::helper::strip_leading_self(path);
            }
        }
        if cyclic.is_empty() {
            push(pred.clone(), &mut out);
            continue;
        }
        let targets = cycle_types_within(&pt.bounded_ty, cycle_self_heads);
        if targets.is_empty() {
            // Nothing to peel to. Left as written so `validate_impl_where_bounds` can report it
            // against the bound the caller actually wrote.
            push(pred.clone(), &mut out);
            continue;
        }
        // The predicate's own `for<'a, …>` binder comes along: a peeled target can mention a bound
        // lifetime (`for<'a> Wrapper<Sub<'a>>: Tr` ⇒ `for<'a> Sub<'a>: Tr`), which would otherwise be
        // undeclared (E0261).
        let binder = &pt.lifetimes;
        for target in targets {
            push(syn::parse_quote!(#binder #target: #(#cyclic)+*), &mut out);
        }
        // An unrelated extra bound (`T: Tr + Clone`) stays on the ORIGINAL type — peeling it too
        // would silently move a constraint onto a different type.
        if !other.is_empty() {
            let bounded = &pt.bounded_ty;
            push(syn::parse_quote!(#binder #bounded: #(#other)+*), &mut out);
        }
    }
    where_clause.predicates = out;
}

/// Is this bound a **bare** (or `self::`-qualified) reference to a trait routed through the engine?
/// A crate-rooted or `super::` spelling is the caller's documented opt-out and never a cycle edge.
fn is_bare_cyclic_bound(bound: &TypeParamBound, all_traits: &HashSet<Ident>) -> bool {
    let TypeParamBound::Trait(TraitBound { path, .. }) = bound else {
        return false;
    };
    let mut path = path.clone();
    crate::helper::strip_leading_self(&mut path);
    path.segments.len() == 1 && all_traits.contains(&path.segments[0].ident)
}

/// Every distinct cycle-member type appearing anywhere inside `ty`, outermost first.
///
/// A member is recognised only by a **bare** (or `self::`-qualified) ident — the same rule
/// `is_bare_cyclic_bound` applies to the trait side. A multi-segment path is an outer type in its
/// entirety, even when its last segment matches a member's name: `crate::other::Stmt` is a
/// different item from the local `Stmt`, and peeling it emitted rank-lowering obligations against
/// a foreign type, which surfaced as a per-rank wall of `TrRanked<((((..` errors (or the
/// "not a type the ranked engine can rank-lower" abort) instead of leaving the bound alone.
///
/// Recursion is unaffected: an outer path's generic arguments are still visited, so a genuine
/// member nested inside a foreign wrapper (`crate::other::Wrap<Stmt>`) is still found.
fn cycle_types_within(ty: &Type, cycle_self_heads: &HashSet<Ident>) -> Vec<Type> {
    struct V<'a> {
        heads: &'a HashSet<Ident>,
        seen: HashSet<String>,
        out: Vec<Type>,
    }
    impl Visit<'_> for V<'_> {
        fn visit_type_path(&mut self, tp: &TypePath) {
            if tp.qself.is_none() && crate::helper::path_names_local_ident(&tp.path, self.heads) {
                // Emit the canonical BARE spelling, exactly as the trait side is normalised above:
                // a peeled target is re-emitted two modules deeper, where a `self::`-qualified
                // path would resolve against the wrong module.
                let mut tp = tp.clone();
                crate::helper::strip_leading_self(&mut tp.path);
                let t = Type::Path(tp);
                if self.seen.insert(quote!(#t).to_string()) {
                    self.out.push(t);
                }
            }
            syn::visit::visit_type_path(self, tp);
        }
    }
    let mut v = V {
        heads: cycle_self_heads,
        seen: HashSet::new(),
        out: Vec::new(),
    };
    v.visit_type(ty);
    v.out
}
