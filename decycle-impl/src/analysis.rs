//! Read-only inspection of a `#[decycle]` module's **obligation graph**.
//!
//! Both engines work out the same thing before they generate anything: which of the module's types
//! participate in a cyclic trait obligation, and which other participant each of their where-bounds
//! reaches. That analysis is useful on its own — for diagnostics, for tooling, or for a caller that
//! wants to decide something before invoking an engine — so it is exposed here as a plain function
//! that generates no code and mutates nothing.
//!
//! The graph is over **type idents**, not over the module's fields: an edge exists because an `impl`
//! *stated* a cyclic where-bound, not because one type happens to contain another. A type with no
//! impl of a routed trait is not a node at all, even if a participant stores it.
//!
//! # Example
//!
//! ```ignore
//! use decycle_impl::analysis::{analyze_module, EdgeKind};
//! use decycle_impl::safegraph::graph::Graph;
//!
//! // mod ast {
//! //     #[decycle] pub trait Tr { fn f(&self); }
//! //     pub enum Expr { … }   impl Tr for Expr where Vec<Box<Stmt>>: Tr { … }
//! //     pub enum Stmt { … }   impl Tr for Stmt where Expr: Tr          { … }
//! // }
//! let g = analyze_module(&module, &parse_quote!(::decycle));
//! // nodes: Expr, Stmt
//! // edges: Expr -Peeled-> Stmt   (the bound was written on `Vec<Box<Stmt>>`)
//! //        Stmt -Direct-> Expr   (the bound named `Expr` itself)
//! ```

use crate::helper::{strip_leading_self, type_head_ident};
use crate::safegraph::graph::Graph;
use crate::safegraph::VecGraph;
use proc_macro2::Ident;
use std::collections::{HashMap, HashSet};
use syn::visit::Visit;
use syn::{Item, ItemImpl, ItemMod, Path, TraitBound, Type, TypeParamBound, UseTree, WherePredicate};

/// How a cyclic where-bound names the participant it reaches.
///
/// The distinction is the ranked engine's: a bound can only be rank-lowered when its target's *head*
/// is a participant, so a wrapped bound has to be peeled to one first (see [`crate::ranked::peel`]).
/// The structural engine draws the same line for a different reason — it *strips* cyclic bounds, and
/// stripping a wrapped one is only sound if the container forwards the trait, so it emits a
/// forwarding assertion in exactly the [`EdgeKind::Peeled`] case.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum EdgeKind {
    /// The bound names the participant directly: `Stmt: Tr` — its head *is* the target.
    Direct,
    /// The bound names a type that *contains* the participant: `Vec<Box<Stmt>>: Tr` — reaching
    /// `Stmt` means looking inside the arguments.
    ///
    /// The two kinds are per *endpoint*, not per bound: one predicate can yield both. When the head
    /// is itself a participant (`Wrap<Stmt>: Tr`, with `Wrap` implementing the trait too) the bound
    /// is a [`EdgeKind::Direct`] edge to `Wrap` **and** a `Peeled` edge to `Stmt`, because the
    /// obligation on `Expr` really does travel through both.
    Peeled,
}

/// Build the obligation graph of `module`.
///
/// Nodes are the head idents of types with an `impl` of a routed trait — the same set the engines
/// call `cycle_self_heads`. Edges go **from** the type carrying the bound **to** the participant the
/// bound reaches, labelled by how it was reached.
///
/// `decycle` is the path the caller passes to an engine (e.g. `::my_crate::__decycle`); only its
/// first segment is used, to recognise `#[<crate>::decycle]` alongside the bare `#[decycle]`. A
/// path with no segments names no crate, so nothing is routed and the graph is empty.
///
/// A bound reaches **every** participant it names: `Wrap<Stmt>: Tr`, with `Wrap` implementing the
/// trait too, is a `Direct` edge to `Wrap` *and* a `Peeled` edge to `Stmt`.
///
/// Not represented, deliberately:
/// - bounds whose target is `Self` or one of the impl's own type parameters — those name no other
///   participant, so there is no second endpoint to draw an edge to;
/// - bounds whose target is a FOREIGN type that merely shares its last path segment with a
///   participant (`crate::other::Stmt: Tr`): a different item, and no engine treats it as a cycle
///   edge either (`ranked::peel::cycle_types_within`). Only the bare and the no-op `self::`
///   spellings name a participant;
/// - the trait each edge came from. Two impls of different routed traits relating the same pair the
///   same way yield **one** edge, since the question this graph answers is about types.
///
/// Two known imprecisions, both kept on purpose so the graph keeps describing what the engines
/// actually do:
/// - a NODE is still the last path segment of the impl's self type, so `impl Tr for
///   ::other::Stmt` contributes the node `Stmt`. That is exactly the engines' own
///   `cycle_self_heads`, and the node set is what [`crate::ranked::process_module_with_graph`]
///   substitutes for it — dropping such a head here would desynchronise the graph from the
///   expansion it is meant to reproduce (a graph over idents cannot spell a foreign path anyway);
/// - a glob `#[decycle] use path::*;` routes no trait, because a glob names none. A module whose
///   only marker is a glob therefore yields the empty graph — the ranked engine rejects that module
///   outright ("glob is not supported in #[decycle] use"), so there is no expansion to describe.
pub fn analyze_module(module: &ItemMod, decycle: &Path) -> VecGraph<Ident, EdgeKind> {
    let Some((_, items)) = module.content.as_ref() else {
        return VecGraph::default();
    };
    // An empty path can only be hand-built, but this is a public API: fail soft, not via `expect`.
    let Some(first_segment) = decycle.segments.first() else {
        return VecGraph::default();
    };
    let decycle_crate = &first_segment.ident;
    let routed = routed_traits(items, decycle_crate);
    build(items, &routed, Spelling::BareOnly)
}

/// Build the obligation graph over `items`, taking the routed traits **as given** rather than
/// discovering them from `#[decycle]` markers, and matching a trait path by its **last segment**.
///
/// This is the entry point for a caller that generates the impls it is about to hand to an engine.
/// Such a caller knows its routed traits, and writes every path fully qualified — so neither the
/// `#[decycle]` markers nor the bare/qualified spelling is available yet, and neither is needed:
/// naming the traits *is* the signal. Contrast [`analyze_module`], which reads a module that has
/// already been spelled for the engine and therefore honours the qualified opt-out.
///
/// Feed the result through [`cyclic_subgraph`] to get the participants, then pass that to
/// [`crate::ranked::process_module_with_graph`] with `emit_contracts` set to `true` — which
/// re-spells the impls from it, closing the loop.
pub fn analyze_items(items: &[Item], routed: &HashSet<Ident>) -> VecGraph<Ident, EdgeKind> {
    build(items, routed, Spelling::AnySpelling)
}

/// How a trait path is matched against the routed set.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Spelling {
    /// Single segment after `self::`-normalisation. A qualified path is the caller's opt-out.
    BareOnly,
    /// Last segment, whatever the qualification. For impls not yet spelled for the engine.
    AnySpelling,
}

fn build(items: &[Item], routed: &HashSet<Ident>, how: Spelling) -> VecGraph<Ident, EdgeKind> {
    let mut graph = VecGraph::default();
    let participants = participants(items, routed, how);

    // Deterministic node order, so the indices a caller sees are reproducible across runs. The
    // idents themselves are CLONED from the source — never re-created via `Ident::new`, which
    // rejects the `to_string()` rendering of a raw identifier (`"r#loop"`).
    let mut node_idents: Vec<Ident> = participants.iter().cloned().collect();
    node_idents.sort_by_key(|i| i.to_string());

    // Collect edges before touching the graph: `scope_mut` hands out scope-local indices, so nodes
    // and edges have to be inserted together inside one scope.
    let mut edges: Vec<(String, String, EdgeKind)> = Vec::new();
    let mut seen: HashSet<(String, String, EdgeKind)> = HashSet::new();
    for item in items {
        let Item::Impl(im) = item else { continue };
        if !impls_routed_trait(im, routed, how) {
            continue;
        }
        let Some(from) = type_head_ident(&im.self_ty) else {
            continue;
        };
        if !participants.contains(&from) {
            continue;
        }
        for (to, kind) in bound_edges(im, routed, &participants, how) {
            let key = (from.to_string(), to.to_string(), kind);
            if seen.insert(key.clone()) {
                edges.push(key);
            }
        }
    }

    // `VecGraph` is `Vec`-backed, so its indices are not stable across removals and `insert_node`
    // is only offered inside a scope. Nothing is removed here, so one scope covers the whole build.
    graph.scope_mut(|mut ctx| {
        let index: HashMap<String, _> = node_idents
            .into_iter()
            .map(|ident| {
                let name = ident.to_string();
                let ix = ctx
                    .insert_node(ident)
                    .expect("insertion into a fresh VecGraph cannot fail");
                (name, ix)
            })
            .collect();
        for (from, to, kind) in &edges {
            ctx.insert_edge(*kind, [index[from], index[to]])
                .expect("both endpoints were just inserted");
        }
    });
    graph
}

/// Trait idents routed through an engine: `#[decycle] trait …` plus every name bound by a
/// `#[decycle] use …`.
fn routed_traits(items: &[Item], decycle_crate: &Ident) -> HashSet<Ident> {
    let mut out = HashSet::new();
    for item in items {
        match item {
            Item::Trait(t) if has_decycle_attr(&t.attrs, decycle_crate) => {
                out.insert(t.ident.clone());
            }
            Item::Use(u) if has_decycle_attr(&u.attrs, decycle_crate) => {
                collect_use_idents(&u.tree, &mut out);
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
fn has_decycle_attr(attrs: &[syn::Attribute], decycle_crate: &Ident) -> bool {
    attrs
        .iter()
        .any(|a| crate::names_decycle_attribute(a, decycle_crate))
}

fn collect_use_idents(tree: &UseTree, out: &mut HashSet<Ident>) {
    match tree {
        UseTree::Path(p) => collect_use_idents(&p.tree, out),
        UseTree::Name(n) => {
            out.insert(n.ident.clone());
        }
        UseTree::Rename(r) => {
            out.insert(r.rename.clone());
        }
        UseTree::Group(g) => g.items.iter().for_each(|t| collect_use_idents(t, out)),
        // A glob names no trait, so it routes none — see the caveat on `analyze_module`. The ranked
        // engine rejects `#[decycle] use path::*;` outright, so a module relying on one has no
        // expansion for this graph to describe.
        UseTree::Glob(_) => {}
    }
}

/// Head idents of the types implementing a routed trait here — the engines' `cycle_self_heads`.
fn participants(items: &[Item], routed: &HashSet<Ident>, how: Spelling) -> HashSet<Ident> {
    items
        .iter()
        .filter_map(|item| {
            let Item::Impl(im) = item else { return None };
            impls_routed_trait(im, routed, how)
                .then(|| type_head_ident(&im.self_ty))
                .flatten()
        })
        .collect()
}

/// Does this impl's trait path name a routed trait in the form the engines adopt — a single segment
/// after `self::`-normalisation? A crate-rooted or `super::` spelling is the documented opt-out.
fn impls_routed_trait(im: &ItemImpl, routed: &HashSet<Ident>, how: Spelling) -> bool {
    let Some((_, path, _)) = im.trait_.as_ref() else {
        return false;
    };
    path_names_routed(path, routed, how)
}

fn path_names_routed(path: &Path, routed: &HashSet<Ident>, how: Spelling) -> bool {
    let mut path = path.clone();
    strip_leading_self(&mut path);
    match how {
        Spelling::BareOnly => path.segments.len() == 1 && routed.contains(&path.segments[0].ident),
        Spelling::AnySpelling => path
            .segments
            .last()
            .is_some_and(|s| routed.contains(&s.ident)),
    }
}

/// The participants reached by this impl's cyclic where-bounds, with how each was reached.
fn bound_edges(
    im: &ItemImpl,
    routed: &HashSet<Ident>,
    participants: &HashSet<Ident>,
    how: Spelling,
) -> Vec<(Ident, EdgeKind)> {
    let Some(where_clause) = im.generics.where_clause.as_ref() else {
        return Vec::new();
    };
    // The impl's own type parameters. A bound on one of them (`impl<Stmt> Tr for Expr where
    // Stmt: Tr`) targets the parameter, not a same-named participant it may shadow — the doc
    // above promises such bounds are not represented, and the engine's own matchers
    // (`remove_cyclic_bounds` / `cyclic_where_bounds` in `ranked/finalize.rs`) already consult
    // exactly this set.
    let param_idents: HashSet<Ident> = im
        .generics
        .params
        .iter()
        .filter_map(|p| match p {
            syn::GenericParam::Type(t) => Some(t.ident.clone()),
            _ => None,
        })
        .collect();
    let mut out = Vec::new();
    for pred in &where_clause.predicates {
        let WherePredicate::Type(pt) = pred else {
            continue;
        };
        if !pt.bounds.iter().any(|b| is_routed_bound(b, routed, how)) {
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
        // the search: `Wrap<Stmt>: Tr` — where `Wrap` implements the trait as well — is a `Direct`
        // edge to `Wrap` *and* a `Peeled` edge to `Stmt`. Treating the two as alternatives dropped
        // the nested endpoints of any bound wrapped in a participating container, which is enough
        // to turn a fully cyclic module (`Expr: Tr <= Wrap<Stmt>: Tr <= Stmt: Tr <= Wrap<Expr>:
        // Tr`) into an acyclic graph — `cyclic_subgraph` then reported NO participants at all, and
        // an engine fed that answer left the module un-decycled.
        let head = local_participant_head(&pt.bounded_ty, participants);
        if let Some(head) = &head {
            out.push((head.clone(), EdgeKind::Direct));
        }
        // `nested_participants` walks the whole type, head included; the head already has its
        // `Direct` edge, so it must not also be reported as reached-from-inside.
        out.extend(
            nested_participants(&pt.bounded_ty, participants)
                .into_iter()
                .filter(|t| head.as_ref() != Some(t))
                .map(|t| (t, EdgeKind::Peeled)),
        );
    }
    out
}

fn is_routed_bound(bound: &TypeParamBound, routed: &HashSet<Ident>, how: Spelling) -> bool {
    let TypeParamBound::Trait(TraitBound { path, .. }) = bound else {
        return false;
    };
    path_names_routed(path, routed, how)
}

/// The participant `path` names — but only when the path can actually *denote* one: a bare ident,
/// or its no-op `self::`-qualified form.
///
/// A path rooted anywhere else (`crate::other::Stmt`, `super::Stmt`, `::dep::Stmt`) reaches a
/// DIFFERENT item that merely shares its last segment with a participant, so a bound on it is an
/// ordinary outer premise and no edge at all. This is the rule the engine already applies when it
/// decides what a bound may be peeled to (`ranked::peel::cycle_types_within`) and when a
/// graph-supplied participant set is spelled back onto the impls (`ranked::contract`); matching the
/// last segment instead made `impl Tr for Expr where ::other::Stmt: Tr` report a cycle
/// `{Expr, Stmt}` over the LOCAL `Stmt` that no engine would ever break.
///
/// The returned ident is cloned from the source, never rebuilt, so a raw identifier survives.
fn local_participant(path: &Path, participants: &HashSet<Ident>) -> Option<Ident> {
    if !crate::helper::path_names_local_ident(path, participants) {
        return None;
    }
    let mut probe = path.clone();
    strip_leading_self(&mut probe);
    probe.segments.last().map(|s| s.ident.clone())
}

/// [`local_participant`] applied to the *head* of `ty` — `None` for a non-path type (`&Stmt`,
/// `(A, B)`) or a `<T as Tr>::X` qself, as [`type_head_ident`] has it.
fn local_participant_head(ty: &Type, participants: &HashSet<Ident>) -> Option<Ident> {
    match ty {
        Type::Path(syn::TypePath { qself: None, path }) => local_participant(path, participants),
        _ => None,
    }
}

/// Participant idents appearing anywhere inside `ty`, outermost first, without repeats.
fn nested_participants(ty: &Type, participants: &HashSet<Ident>) -> Vec<Ident> {
    struct V<'a> {
        participants: &'a HashSet<Ident>,
        seen: HashSet<String>,
        out: Vec<Ident>,
    }
    impl Visit<'_> for V<'_> {
        fn visit_type_path(&mut self, tp: &syn::TypePath) {
            if tp.qself.is_none() {
                if let Some(ident) = local_participant(&tp.path, self.participants) {
                    if self.seen.insert(ident.to_string()) {
                        self.out.push(ident);
                    }
                }
            }
            // A foreign wrapper still has its arguments visited: `crate::other::Wrap<Stmt>`
            // reaches the local `Stmt`.
            syn::visit::visit_type_path(self, tp);
        }
    }
    let mut v = V {
        participants,
        seen: HashSet::new(),
        out: Vec::new(),
    };
    v.visit_type(ty);
    v.out
}

/// Read a graph back out as `(nodes, (from, to, kind) edges)`, nodes in enumeration order and
/// edges referring to nodes by **position** in that order — not by name, so two same-named nodes
/// in a caller-built graph stay distinct through a round-trip.
///
/// Indices of a `Vec`-backed graph only exist inside a scope, so anything that inspects one has to
/// do it here and hand out owned data.
fn decompose(graph: &VecGraph<Ident, EdgeKind>) -> (Vec<Ident>, Vec<(usize, usize, EdgeKind)>) {
    graph.scope(|ctx| {
        let order: Vec<_> = ctx.node_indices().collect();
        let pos: HashMap<_, usize> = order.iter().enumerate().map(|(i, ix)| (*ix, i)).collect();
        let nodes: Vec<Ident> = order.iter().map(|ix| ctx.node(*ix).clone()).collect();
        let edges = ctx
            .edge_indices()
            .map(|e| {
                let [a, b] = ctx.endpoints(e);
                (pos[&a], pos[&b], *ctx.edge(e))
            })
            .collect();
        (nodes, edges)
    })
}

/// Assemble a graph from nodes and `(from, to, kind)` edges given by node **position**. Edges
/// referring to an absent position are dropped; nodes are inserted in the order given.
fn compose(nodes: Vec<Ident>, edges: Vec<(usize, usize, EdgeKind)>) -> VecGraph<Ident, EdgeKind> {
    let mut out = VecGraph::default();
    out.scope_mut(|mut ctx| {
        let index: Vec<_> = nodes
            .into_iter()
            .map(|n| {
                ctx.insert_node(n)
                    .expect("insertion into a fresh VecGraph cannot fail")
            })
            .collect();
        for (from, to, kind) in edges {
            if let (Some(a), Some(b)) = (index.get(from), index.get(to)) {
                ctx.insert_edge(kind, [*a, *b])
                    .expect("both endpoints exist");
            }
        }
    });
    out
}

/// Restrict `graph` to the nodes that actually **lie on a cycle**, keeping the edges between them.
///
/// A node is retained when it belongs to a strongly connected component of more than one node, or
/// when it has a self-edge. A lone node with no self-edge is not recursive, however many other nodes
/// point at it.
///
/// This is the counterpart to building a graph from a reference relation: a caller can hand over
/// *everything* it knows — every type and every reference between them — and let this decide which of
/// them recurse, rather than implementing a reachability search of its own. The result is in the
/// shape [`crate::ranked::process_module_with_graph`] wants.
pub fn cyclic_subgraph(graph: &VecGraph<Ident, EdgeKind>) -> VecGraph<Ident, EdgeKind> {
    // Everything is computed inside ONE scope, keyed by node position rather than by name, so a
    // caller-built graph with two same-named nodes keeps them distinct throughout.
    let (nodes, edges, cyclic) = graph.scope(|ctx| {
        let order: Vec<_> = ctx.node_indices().collect();
        let pos: HashMap<_, usize> = order.iter().enumerate().map(|(i, ix)| (*ix, i)).collect();
        let nodes: Vec<Ident> = order.iter().map(|ix| ctx.node(*ix).clone()).collect();
        let edges: Vec<(usize, usize, EdgeKind)> = ctx
            .edge_indices()
            .map(|e| {
                let [a, b] = ctx.endpoints(e);
                (pos[&a], pos[&b], *ctx.edge(e))
            })
            .collect();
        let self_looped: HashSet<usize> = edges
            .iter()
            .filter(|(a, b, _)| a == b)
            .map(|(a, _, _)| *a)
            .collect();
        let mut cyclic: HashSet<usize> = HashSet::new();
        for component in crate::safegraph::algo::connectivity::tarjan_scc(ctx) {
            let members: Vec<usize> = component.iter().map(|ix| pos[ix]).collect();
            if members.len() > 1 || members.iter().any(|p| self_looped.contains(p)) {
                cyclic.extend(members);
            }
        }
        (nodes, edges, cyclic)
    });

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
        .map(|(_, n)| n)
        .collect();
    let edges = edges
        .into_iter()
        .filter_map(|(a, b, kind)| Some((*remap.get(&a)?, *remap.get(&b)?, kind)))
        .collect();
    compose(nodes, edges)
}

/// `graph` plus `extra` nodes, edges unchanged.
///
/// For a caller that discovers further participants only after the graph is built — a macro that
/// generates a helper type belonging to the cycle, say. Names already present are not duplicated.
pub fn with_nodes(
    graph: &VecGraph<Ident, EdgeKind>,
    extra: impl IntoIterator<Item = Ident>,
) -> VecGraph<Ident, EdgeKind> {
    let (mut nodes, edges) = decompose(graph);
    let mut have: HashSet<String> = nodes.iter().map(|n| n.to_string()).collect();
    for name in extra {
        if have.insert(name.to_string()) {
            nodes.push(name);
        }
    }
    compose(nodes, edges)
}
