//! A `#[decycle]` module's **obligation graph**, read without generating anything.
//!
//! Which of the module's types participate in a cyclic trait obligation, and which other
//! participant each of their where-bounds reaches. Crate-internal: `crate::growth` uses it to find
//! the cycle members before either engine runs.
//!
//! The graph is over **type idents**, not over the module's fields: an edge exists because an `impl`
//! *stated* a cyclic where-bound, not because one type happens to contain another. A type with no
//! impl of a routed trait is not a node at all, even if a participant stores it.
//!
//! ```text
//! mod ast {
//!     #[decycle] pub trait Tr { fn f(&self); }
//!     pub enum Expr { … }   impl Tr for Expr where Vec<Box<Stmt>>: Tr { … }
//!     pub enum Stmt { … }   impl Tr for Stmt where Expr: Tr          { … }
//! }
//! nodes: Expr, Stmt
//! edges: Expr -> Stmt   (the bound was written on `Vec<Box<Stmt>>`)
//!        Stmt -> Expr   (the bound named `Expr` itself)
//! ```

use crate::names::{bare_ident, is_routed_bound};
use proc_macro2::Ident;
use safegraph::VecGraph;
use std::collections::{HashMap, HashSet};
use syn::{Attribute, Item, ItemImpl, ItemMod, Path, Type};

/// Build the obligation graph of `module`.
///
/// Nodes are the head idents of types with an `impl` of a routed trait — the same set the engines
/// call `cycle_self_heads`. Edges go **from** the type carrying the bound **to** the participant the
/// bound reaches, whether the bound names it (`Stmt: Tr`) or a type containing it
/// (`Vec<Box<Stmt>>: Tr`).
///
/// `decycle` is the path the caller passes to an engine (e.g. `::my_crate::__decycle`); only its
/// first segment is used, to recognise `#[<crate>::decycle]` alongside the bare `#[decycle]`. A
/// path with no segments names no crate, so nothing is routed and the graph is empty.
///
/// A bound reaches **every** participant it names: `Wrap<Stmt>: Tr`, with `Wrap` implementing the
/// trait too, is an edge to `Wrap` *and* an edge to `Stmt`.
///
/// Not represented, deliberately:
/// - bounds whose target is `Self` or one of the impl's own type parameters — those name no other
///   participant, so there is no second endpoint to draw an edge to;
/// - bounds whose target is a FOREIGN type that merely shares its last path segment with a
///   participant (`crate::other::Stmt: Tr`): a different item, and no engine treats it as a cycle
///   edge either (`ranked::peel::cycle_types_within`). Only the bare and the no-op `self::`
///   spellings name a participant;
/// - the trait each edge came from. Two impls of different routed traits relating the same pair the
///   same way yield **one** edge, since the question this graph answers is about types;
/// - impls whose self type the module spells by a path (`impl Tr for crate::m::Term`, `impl Tr for
///   ::other::Stmt`): like a path-spelled trait, that names the item itself, the opt-out, and the
///   engines take no such impl over. Only a bare (or `self::`) self type contributes a node, so the
///   node set stays exactly the engines' own `cycle_self_heads`.
///
/// One known imprecision, kept on purpose so the graph keeps describing what the engines actually
/// do: a glob `#[decycle] use path::*;` routes no trait, because a glob names none. A module whose
/// only marker is a glob therefore yields the empty graph — the ranked engine rejects that module
/// outright ("glob is not supported in #[decycle] use"), so there is no expansion to describe.
pub(crate) fn analyze_module(module: &ItemMod, decycle: &Path) -> VecGraph<Ident, ()> {
    let Some((_, items)) = module.content.as_ref() else {
        return VecGraph::default();
    };
    // An empty path can only be hand-built: fail soft, not via `expect`.
    let Some(first_segment) = decycle.segments.first() else {
        return VecGraph::default();
    };
    let decycle_crate = &first_segment.ident;
    let routed = routed_traits(items, &|attrs| has_decycle_attr(attrs, decycle_crate));
    build(items, &routed)
}

fn build(items: &[Item], routed: &HashSet<Ident>) -> VecGraph<Ident, ()> {
    let participants = participants(items, routed);

    // Deterministic node order, so the indices a caller sees are reproducible across runs. The
    // idents themselves are CLONED from the source — never re-created via `Ident::new`, which
    // rejects the `to_string()` rendering of a raw identifier (`"r#loop"`).
    let mut node_idents: Vec<Ident> = participants.iter().cloned().collect();
    node_idents.sort_by_key(|i| i.to_string());
    let position: HashMap<String, usize> = node_idents
        .iter()
        .enumerate()
        .map(|(k, ident)| (ident.to_string(), k))
        .collect();

    let mut edges: Vec<(usize, usize, ())> = Vec::new();
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    for item in items {
        let Item::Impl(im) = item else { continue };
        if !impls_routed_trait(im, routed) {
            continue;
        }
        let Some(from) = self_head(im) else {
            continue;
        };
        if !participants.contains(&from) {
            continue;
        }
        for to in bound_edges(im, routed, &participants) {
            let key = (position[&from.to_string()], position[&to.to_string()]);
            if seen.insert(key) {
                edges.push((key.0, key.1, ()));
            }
        }
    }
    crate::graph::compose(node_idents, edges)
}

/// Trait idents routed through an engine: `#[decycle] trait …` plus every name bound by a
/// `#[decycle] use …`, by its local name; `marked` says whether an item's attributes carry the
/// marker.
///
/// A glob names no trait, so it routes none — see the caveat on `analyze_module`. The ranked engine
/// rejects `#[decycle] use path::*;` outright, so a module relying on one has no expansion for this
/// graph to describe.
pub(crate) fn routed_traits(
    items: &[Item],
    marked: &dyn Fn(&[Attribute]) -> bool,
) -> HashSet<Ident> {
    let mut out = HashSet::new();
    for item in items {
        match item {
            Item::Trait(t) if marked(&t.attrs) => {
                out.insert(t.ident.clone());
            }
            Item::Use(u) if marked(&u.attrs) => {
                out.extend(crate::names::use_names(&u.tree).cloned());
            }
            _ => {}
        }
    }
    out
}

/// The PATH-only predicate, on purpose: reading a module must not diagnose it. An argument list on
/// an inner `#[decycle]` is a hard error in both engines (`crate::is_decycle_attribute`), but that
/// rejection goes through `abort!`, which panics outside a proc-macro entry point — and this
/// function is a plain library call. So the marker is honoured here whatever it carries, and the
/// engine the caller goes on to invoke is the one that reports it.
fn has_decycle_attr(attrs: &[Attribute], decycle_crate: &Ident) -> bool {
    attrs
        .iter()
        .any(|a| crate::names_decycle_attribute(a, Some(decycle_crate)))
}

/// Head idents of the types implementing a routed trait here — the engines' `cycle_self_heads`.
fn participants(items: &[Item], routed: &HashSet<Ident>) -> HashSet<Ident> {
    items
        .iter()
        .filter_map(|item| {
            let Item::Impl(im) = item else { return None };
            impls_routed_trait(im, routed)
                .then(|| self_head(im))
                .flatten()
        })
        .collect()
}

/// The head ident of the impl's self type, as a node — read as the engines do: a self type spelled
/// by a path (`impl Tr for crate::m::Term`) names the item itself, the opt-out, and is no node.
fn self_head(im: &ItemImpl) -> Option<Ident> {
    crate::names::local_type_head_ident(&im.self_ty)
}

/// Does this impl's trait path name a routed trait in the form the engines adopt — bare, or
/// `self::` ([`bare_ident`])? Any other spelling (`crate::m::Tr`, `super::Tr`, `::Tr`) is the
/// documented opt-out.
fn impls_routed_trait(im: &ItemImpl, routed: &HashSet<Ident>) -> bool {
    im.trait_
        .as_ref()
        .and_then(|(_, path, _)| bare_ident(path))
        .is_some_and(|ident| routed.contains(ident))
}

/// The participants reached by this impl's cyclic where-bounds.
fn bound_edges(
    im: &ItemImpl,
    routed: &HashSet<Ident>,
    participants: &HashSet<Ident>,
) -> Vec<Ident> {
    // The impl's own type parameters. A bound on one of them (`impl<Stmt> Tr for Expr where
    // Stmt: Tr`) targets the parameter, not a same-named participant it may shadow — the doc
    // above promises such bounds are not represented, and the engine's own matchers
    // (`remove_cyclic_bounds` / `cyclic_where_bounds` in `ranked/finalize.rs`) already consult
    // exactly this set.
    let param_idents = crate::names::type_param_idents(&im.generics);
    let mut out = Vec::new();
    for pt in crate::names::type_predicates(&im.generics) {
        if !pt.bounds.iter().any(|b| is_routed_bound(b, routed)) {
            continue;
        }
        // Target is `Self` or a bare impl type param: no second endpoint, no edge (documented).
        if let Type::Path(syn::TypePath { qself: None, path }) = &pt.bounded_ty {
            if path.is_ident("Self")
                || (path.segments.len() == 1 && param_idents.contains(&path.segments[0].ident))
            {
                continue;
            }
        }
        // A bound reaches EVERY participant it names, and the head being one of them does not stop
        // the search: `Wrap<Stmt>: Tr` — where `Wrap` implements the trait as well — is an edge to
        // `Wrap` *and* to `Stmt`. Treating the two as alternatives dropped the nested endpoints of
        // any bound wrapped in a participating container, which is enough to turn a fully cyclic
        // module (`Expr: Tr <= Wrap<Stmt>: Tr <= Stmt: Tr <= Wrap<Expr>: Tr`) into an acyclic graph
        // — `cyclic_subgraph` then reported NO participants at all, and an engine fed that answer
        // left the module un-decycled.
        out.extend(nested_participants(&pt.bounded_ty, participants));
    }
    out
}

/// Participant idents appearing anywhere inside `ty`, its head included, outermost first, without
/// repeats — each only where its path can actually *denote* one: a bare ident, or its no-op
/// `self::`-qualified form (`crate::names::bare_ident`).
///
/// A path rooted anywhere else (`crate::other::Stmt`, `super::Stmt`, `::dep::Stmt`) reaches a
/// DIFFERENT item that merely shares its last segment with a participant, so a bound on it is an
/// ordinary outer premise and no edge at all. This is the rule the engine already applies when it
/// decides what a bound may be peeled to (`ranked::peel::cycle_types_within`); matching the last
/// segment instead made `impl Tr for Expr where ::other::Stmt: Tr` report a cycle `{Expr, Stmt}`
/// over the LOCAL `Stmt` that no engine would ever break. A foreign wrapper still has its
/// arguments visited: `crate::other::Wrap<Stmt>` reaches the local `Stmt`.
///
/// The returned idents are cloned from the source, never rebuilt, so a raw identifier survives.
fn nested_participants(ty: &Type, participants: &HashSet<Ident>) -> Vec<Ident> {
    let mut seen: HashSet<String> = HashSet::new();
    crate::names::bare_named_in(ty, &|i| participants.contains(i))
        .into_iter()
        .filter_map(|tp| crate::names::bare_ident(&tp.path))
        .filter(|ident| seen.insert(ident.to_string()))
        .cloned()
        .collect()
}

/// Restrict `graph` to the nodes that actually **lie on a cycle**, keeping the edges between them.
///
/// A node is retained when it belongs to a strongly connected component of more than one node, or
/// when it has a self-edge. A lone node with no self-edge is not recursive, however many other nodes
/// point at it.
///
pub(crate) fn cyclic_subgraph(graph: &VecGraph<Ident, ()>) -> VecGraph<Ident, ()> {
    // Keyed by node position rather than by name, so a caller-built graph with two same-named
    // nodes keeps them distinct throughout.
    let (nodes, edges) = crate::graph::decompose(graph);
    let cyclic: HashSet<usize> = crate::graph::cyclic_components(graph)
        .into_iter()
        .flatten()
        .collect();

    // Retained nodes keep their relative order; edge endpoints are remapped into that subsequence.
    let remap: HashMap<usize, usize> = (0..nodes.len())
        .filter(|i| cyclic.contains(i))
        .enumerate()
        .map(|(new, old)| (old, new))
        .collect();
    let nodes = nodes
        .into_iter()
        .enumerate()
        .filter(|(i, _)| cyclic.contains(i))
        .map(|(_, n)| n);
    let edges = edges
        .into_iter()
        .filter_map(|(a, b, ())| Some((*remap.get(&a)?, *remap.get(&b)?, ())));
    crate::graph::compose(nodes, edges)
}

#[cfg(test)]
mod tests;
