//! Normalise a caller's impls into the spelling the ranked engine reads, using a supplied graph.
//!
//! The engine classifies by **spelling**: a bare single-segment trait path is a cycle edge to rank, a
//! crate-rooted one is an ordinary premise to leave alone. That convention exists because, without
//! more information, spelling is the only channel a caller has to say which is which — so a caller
//! generating impls had to encode its own cycle analysis into how it wrote every bound, and get it
//! right, in its own crate.
//!
//! A caller that supplies a graph has already said it directly. This pass therefore does the
//! encoding *here*: given the participant set, it re-spells each adopted impl so the classification
//! the engine reads back is the one the graph states.
//!
//! - the impl's **header** trait path becomes the bare ident, which is what makes `process_module`
//!   adopt it at all;
//! - a where-bound whose target **reaches a participant** is spelled bare — a cycle edge;
//! - every other bound naming a routed trait is spelled **fully qualified** — a premise. This matters
//!   even though the caller probably wrote it that way already: a bound copied out of a user's own
//!   `where`-clause may well be bare (`where Integer: Parse<A>`, with `Parse` imported), and left
//!   bare it would reach the engine as a would-be edge on a non-participant head.
//!
//! Only bounds naming the impl's **own** trait can become edges. A bound naming a *different* routed
//! trait is always qualified, even when its target is a participant: it is a premise of this impl, not
//! an edge of the cycle being ranked here, and baring it would have `TraitReplacer` rewrite it to the
//! wrong ranked twin.

use crate::helper::type_head_ident;
use proc_macro2::Ident;
use std::collections::{HashMap, HashSet};
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{
    Item, ItemImpl, Path, PathSegment, Token, TraitBound, Type, TypeParamBound, TypePath,
    WherePredicate,
};
use template_quote::quote;

/// Re-spell every impl the graph marks as a cycle member.
///
/// `participants` are the graph's nodes. An impl is a cycle member when its self type's head is one
/// *and* its trait path's last segment names a routed trait — note **last segment**, not
/// single-segment: the whole point is that the caller has not normalised the spelling yet.
pub(crate) fn contract_from_graph(
    contents: &mut [Item],
    all_traits: &HashSet<Ident>,
    participants: &HashSet<Ident>,
) {
    // The qualified form of each routed trait, recovered from the impl headers the caller wrote. This
    // is what lets a leaf bound be re-qualified without the caller having to pass the paths in.
    let qualified = collect_qualified_paths(contents, all_traits);

    for item in contents.iter_mut() {
        let Item::Impl(im) = item else { continue };
        let Some(own) = own_routed_trait(im, all_traits) else {
            continue;
        };
        // `map_or(true, …)` rather than `is_none_or`: the latter is stable only since 1.82 and
        // this crate's MSRV is 1.71.
        if type_head_ident(&im.self_ty).map_or(true, |h| !participants.contains(&h)) {
            continue;
        }
        bare_header(im, &own);
        respell_bounds(im, &own, all_traits, participants, &qualified);
    }
}

/// The routed trait this impl implements, by the last segment of its trait path.
fn own_routed_trait(im: &ItemImpl, all_traits: &HashSet<Ident>) -> Option<Ident> {
    let (_, path, _) = im.trait_.as_ref()?;
    let last = path.segments.last()?;
    all_traits.contains(&last.ident).then(|| last.ident.clone())
}

/// `Parse` → `::syan::parse::parse::Parse`, taken from the first multi-segment header that names it.
fn collect_qualified_paths(contents: &[Item], all_traits: &HashSet<Ident>) -> HashMap<Ident, Path> {
    let mut out = HashMap::new();
    for item in contents {
        let Item::Impl(im) = item else { continue };
        let Some((_, path, _)) = im.trait_.as_ref() else {
            continue;
        };
        let Some(last) = path.segments.last() else {
            continue;
        };
        if path.segments.len() < 2 || !all_traits.contains(&last.ident) {
            continue;
        }
        let mut bare_path = path.clone();
        // Store the path without arguments; each use site re-attaches its own.
        if let Some(seg) = bare_path.segments.last_mut() {
            seg.arguments = syn::PathArguments::None;
        }
        out.entry(last.ident.clone()).or_insert(bare_path);
    }
    out
}

/// Header trait path → the bare ident, keeping its arguments (`<Atom>`).
fn bare_header(im: &mut ItemImpl, own: &Ident) {
    if let Some((_, path, _)) = &mut im.trait_ {
        let args = path
            .segments
            .last()
            .map(|s| s.arguments.clone())
            .unwrap_or_default();
        *path = single_segment(own.clone(), args);
    }
}

fn respell_bounds(
    im: &mut ItemImpl,
    own: &Ident,
    all_traits: &HashSet<Ident>,
    participants: &HashSet<Ident>,
    qualified: &HashMap<Ident, Path>,
) {
    let Some(where_clause) = &mut im.generics.where_clause else {
        return;
    };
    let mut out: Punctuated<WherePredicate, Token![,]> = Punctuated::new();
    let mut seen: HashSet<String> = HashSet::new();
    for pred in where_clause.predicates.iter() {
        let mut pred = pred.clone();
        if let WherePredicate::Type(pt) = &mut pred {
            // A cycle edge is a bound of THIS trait whose target reaches a participant. The target is
            // left exactly as written, wrappers and all (`Vec<Box<Stmt<S>>>: Parse<A>`) — peeling it
            // to a rankable head is `super::peel`'s job.
            let edge = reaches_participant(&pt.bounded_ty, participants);
            for bound in pt.bounds.iter_mut() {
                let TypeParamBound::Trait(TraitBound { path, .. }) = bound else {
                    continue;
                };
                let Some(last) = path.segments.last() else {
                    continue;
                };
                let named = last.ident.clone();
                if !all_traits.contains(&named) {
                    continue;
                }
                let args = last.arguments.clone();
                if edge && named == *own {
                    *path = single_segment(named, args);
                } else if let Some(full) = qualified.get(&named) {
                    let mut full = full.clone();
                    if let Some(seg) = full.segments.last_mut() {
                        seg.arguments = args;
                    }
                    *path = full;
                }
                // With no qualified form on record the spelling is left as the caller wrote it —
                // re-spelling it to something unresolvable would be worse than leaving it.
            }
        }
        if seen.insert(quote!(#pred).to_string()) {
            out.push(pred);
        }
    }
    where_clause.predicates = out;
}

fn single_segment(ident: Ident, arguments: syn::PathArguments) -> Path {
    Path {
        leading_colon: None,
        segments: core::iter::once(PathSegment { ident, arguments }).collect(),
    }
}

/// Does `ty` mention a participant — at its head or anywhere inside its arguments?
fn reaches_participant(ty: &Type, participants: &HashSet<Ident>) -> bool {
    struct V<'a> {
        participants: &'a HashSet<Ident>,
        hit: bool,
    }
    impl Visit<'_> for V<'_> {
        fn visit_type_path(&mut self, tp: &TypePath) {
            if tp.qself.is_none()
                && tp
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| self.participants.contains(&s.ident))
            {
                self.hit = true;
            }
            syn::visit::visit_type_path(self, tp);
        }
    }
    let mut v = V {
        participants,
        hit: false,
    };
    v.visit_type(ty);
    v.hit
}
