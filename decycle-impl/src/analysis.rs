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
use proc_macro2::{Ident, Span};
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
    /// The bound names a type that *contains* the participant: `Vec<Box<Stmt>>: Tr`. The head
    /// (`Vec`) is not itself a participant, so reaching `Stmt` means looking inside the arguments.
    Peeled,
}

/// Build the obligation graph of `module`.
///
/// Nodes are the head idents of types with an `impl` of a routed trait — the same set the engines
/// call `cycle_self_heads`. Edges go **from** the type carrying the bound **to** the participant the
/// bound reaches, labelled by how it was reached.
///
/// `decycle` is the path the caller passes to an engine (e.g. `::my_crate::__decycle`); only its
/// first segment is used, to recognise `#[<crate>::decycle]` alongside the bare `#[decycle]`.
///
/// Not represented, deliberately:
/// - bounds whose target is `Self` or one of the impl's own type parameters — those name no other
///   participant, so there is no second endpoint to draw an edge to;
/// - the trait each edge came from. Two impls of different routed traits relating the same pair the
///   same way yield **one** edge, since the question this graph answers is about types.
pub fn analyze_module(module: &ItemMod, decycle: &Path) -> VecGraph<Ident, EdgeKind> {
    let Some((_, items)) = module.content.as_ref() else {
        return VecGraph::default();
    };
    let decycle_crate = &decycle.segments.first().expect("empty decycle path").ident;
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
/// [`crate::ranked::process_module_with_graph`] with `contract: true` — which re-spells the impls from
/// it, closing the loop.
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

    // Deterministic node order, so the indices a caller sees are reproducible across runs.
    let mut names: Vec<String> = participants.iter().map(|i| i.to_string()).collect();
    names.sort();

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
        let index: HashMap<String, _> = names
            .iter()
            .map(|name| {
                let ix = ctx
                    .insert_node(Ident::new(name, Span::call_site()))
                    .expect("insertion into a fresh VecGraph cannot fail");
                (name.clone(), ix)
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

fn has_decycle_attr(attrs: &[syn::Attribute], decycle_crate: &Ident) -> bool {
    attrs
        .iter()
        .any(|a| crate::is_decycle_attribute(a, decycle_crate))
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
    let mut out = Vec::new();
    for pred in &where_clause.predicates {
        let WherePredicate::Type(pt) = pred else {
            continue;
        };
        if !pt.bounds.iter().any(|b| is_routed_bound(b, routed, how)) {
            continue;
        }
        match type_head_ident(&pt.bounded_ty) {
            // The head is itself a participant — the bound names it outright.
            Some(head) if participants.contains(&head) => out.push((head, EdgeKind::Direct)),
            // Otherwise look inside: a container may still be carrying one.
            _ => out.extend(
                nested_participants(&pt.bounded_ty, participants)
                    .into_iter()
                    .map(|t| (t, EdgeKind::Peeled)),
            ),
        }
    }
    out
}

fn is_routed_bound(bound: &TypeParamBound, routed: &HashSet<Ident>, how: Spelling) -> bool {
    let TypeParamBound::Trait(TraitBound { path, .. }) = bound else {
        return false;
    };
    path_names_routed(path, routed, how)
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
                if let Some(seg) = tp.path.segments.last() {
                    if self.participants.contains(&seg.ident)
                        && self.seen.insert(seg.ident.to_string())
                    {
                        self.out.push(seg.ident.clone());
                    }
                }
            }
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

/// Read a graph back out as `(node names, (from, to, kind) edges)`, both in insertion order.
///
/// Indices of a `Vec`-backed graph only exist inside a scope, so anything that inspects one has to
/// do it here and hand out owned data.
fn decompose(
    graph: &VecGraph<Ident, EdgeKind>,
) -> (Vec<Ident>, Vec<(String, String, EdgeKind)>) {
    graph.scope(|ctx| {
        let nodes: Vec<Ident> = ctx.node_indices().map(|n| ctx.node(n).clone()).collect();
        let edges = ctx
            .edge_indices()
            .map(|e| {
                let [a, b] = ctx.endpoints(e);
                (
                    ctx.node(a).to_string(),
                    ctx.node(b).to_string(),
                    *ctx.edge(e),
                )
            })
            .collect();
        (nodes, edges)
    })
}

/// Assemble a graph from node names and `(from, to, kind)` edges. Edges naming an absent node are
/// dropped; nodes are inserted in the order given.
fn compose(nodes: Vec<Ident>, edges: Vec<(String, String, EdgeKind)>) -> VecGraph<Ident, EdgeKind> {
    let mut out = VecGraph::default();
    out.scope_mut(|mut ctx| {
        let index: HashMap<String, _> = nodes
            .into_iter()
            .map(|n| {
                let key = n.to_string();
                let ix = ctx
                    .insert_node(n)
                    .expect("insertion into a fresh VecGraph cannot fail");
                (key, ix)
            })
            .collect();
        for (from, to, kind) in edges {
            if let (Some(a), Some(b)) = (index.get(&from), index.get(&to)) {
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
    let (nodes, edges) = decompose(graph);
    let self_looped: HashSet<String> = edges
        .iter()
        .filter(|(a, b, _)| a == b)
        .map(|(a, _, _)| a.clone())
        .collect();

    let cyclic: HashSet<String> = graph.scope(|ctx| {
        let mut out = HashSet::new();
        for component in crate::safegraph::algo::connectivity::tarjan_scc(ctx) {
            let names: Vec<String> = component
                .iter()
                .map(|ix| ctx.node(*ix).to_string())
                .collect();
            if names.len() > 1 || names.iter().any(|n| self_looped.contains(n)) {
                out.extend(names);
            }
        }
        out
    });

    let nodes = nodes
        .into_iter()
        .filter(|n| cyclic.contains(&n.to_string()))
        .collect();
    let edges = edges
        .into_iter()
        .filter(|(a, b, _)| cyclic.contains(a) && cyclic.contains(b))
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
