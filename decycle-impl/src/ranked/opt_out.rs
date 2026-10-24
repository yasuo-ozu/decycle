//! Report an opt-out bound that is on a cycle, in the user's terms, before rustc overflows on it.
//!
//! A bound that names one of the module's own types by a path (`where crate::m::C: Loop`, or the
//! part `crate::m::C` of a peeled `(B, crate::m::C): Loop`) is the opt-out: the engine leaves it as
//! written, an ordinary premise on the original trait. When `C`'s impl leads back to the impl that
//! carries the bound, the premise is a plain trait cycle again, and rustc overflows on it (`E0275`)
//! — reporting an obligation on the engine's generated `…Ranked` trait, which names nothing the
//! user wrote. This pass predicts that overflow and reports it at the bound instead.
//!
//! # When it reports
//!
//! Only with `module = path`, which is what proves that `crate::m::C` is this module's `C`
//! ([`ModuleScope::own_type_by_path`]); without it nothing is reported. And only when the overflow
//! is certain, because an error here must never reject a program that compiles. The pass therefore
//! reads a deliberately small graph, one per `#[decycle]` trait:
//!
//! - **nodes** are the impls the engine takes over (trait and self type bare, or `self::`) that are
//!   concrete: no generic parameters at all, a self type that is a struct, enum or union declared in
//!   the module, named without arguments, and the only impl of the trait for that type. An impl with
//!   any attribute other than a lint or a doc comment (a `#[cfg]`, an attribute macro) is no node,
//!   since it may not exist, or not as written, once it expands.
//! - **edges** are the bounds of a node's `where` clause that name the same trait, bare, on another
//!   node: a bare type (`B: Loop`) is a cycle edge, which the engine ranks; a path to an own type
//!   (`crate::m::C: Loop`) is an opt-out edge, which it leaves as written.
//!
//! A bound on anything else (a generic impl such as `impl<T> Loop for Wrap<T> where T: Loop`, a type
//! with arguments, another trait, an associated-type constraint, a `for<'a>` binder) is no edge, so
//! a chain through it is never judged: whether it leads back can depend on the instantiation, and
//! the pass leaves it to rustc.
//!
//! Every predicate on a concrete chain is *global* (it names no generic parameter), and rustc proves
//! a global `where` predicate where it is written, from no assumptions. So an opt-out edge from `A`
//! to `C` overflows exactly when proving `C: Loop` from `C`'s impl reaches `A`'s impl again, which
//! carries the same premise. On the engine's rank chain that is a reachability question with a
//! depth: the original trait enters `C`'s rank chain at the top rank, `recurse_level`; each cycle
//! edge descends one rank, and the floor (rank 0) has none; an opt-out edge, a premise, is kept at
//! every rank, the floor included, and re-enters its target at the top. A chain of more cycle
//! edges than `recurse_level` reaches the floor before it reaches `A`, and then the program
//! compiles, so nothing is reported (a call that reaches the floor on such a chain fails closed at
//! run time, "re-entry fn not registered"; this pass does not change that). The search runs over
//! `(node, rank)` pairs to tell the two apart.
//!
//! The module's own attributes must be inert too (a lint level, a doc comment): a module under a
//! `#[cfg]` that is off compiles whatever it contains.
//!
//! # What the user then sees
//!
//! One error per such bound, at the type as written, naming the cycle by the module's types
//! (`A` → `C` → `A`), with the bare spelling as the fix. The module is then emitted without the
//! engine (the abort's fallback), where the impls would overflow on their own cycles, so the
//! fallback drops the global bounds of the `#[decycle]` traits ([`quiet`]): the impls hold
//! unconditionally, the bodies still type-check, and the new errors stand alone.

use crate::module_path::ModuleScope;
use crate::names::strip_leading_self;
use std::collections::{BTreeMap, HashMap, HashSet};
use syn::visit::Visit;
use syn::{
    Attribute, GenericParam, Ident, Item, ItemImpl, Path, TraitBound, TraitBoundModifier, Type,
    TypeParamBound, TypePath, WherePredicate,
};

/// An opt-out bound on a cycle, as [`opt_out_cycles`] reports it.
pub(crate) struct OptOutCycle {
    /// The type the bound names, as written (`crate::m::C`): the span of the error.
    pub(crate) at: Type,
    pub(crate) message: String,
    pub(crate) help: String,
}

/// Every opt-out bound of `contents` that is certain to overflow; see the module docs.
///
/// `contents` are the module's items after peeling (so the part of a wrapped bound that the engine
/// keeps as written is a predicate of its own), `routed` the `#[decycle]` traits, and
/// `recurse_level` the engine's top rank. The result is in source order.
pub(crate) fn opt_out_cycles(
    contents: &[Item],
    scope: &ModuleScope,
    routed: &HashSet<Ident>,
    recurse_level: usize,
) -> Vec<OptOutCycle> {
    let impls: Vec<&ItemImpl> = contents
        .iter()
        .filter_map(|item| match item {
            Item::Impl(im) => Some(im),
            _ => None,
        })
        .collect();
    // (trait, self type) of every impl the engine takes over, with whether it is a concrete node.
    // A pair with more than one impl (only possible when `#[cfg]` picks one) is no node.
    let mut candidates: BTreeMap<(String, String), Vec<(bool, &ItemImpl)>> = BTreeMap::new();
    for im in &impls {
        let Some(trait_name) = adopted_trait(im, routed) else {
            continue;
        };
        let Some(head) = crate::names::local_type_head_ident(&im.self_ty) else {
            continue;
        };
        let concrete = is_concrete(im, &head, scope);
        candidates
            .entry((trait_name, crate::helper::unraw(&head)))
            .or_default()
            .push((concrete, im));
    }
    let mut graphs: BTreeMap<String, Graph> = BTreeMap::new();
    for ((trait_name, head), entries) in &candidates {
        if let [(true, im)] = entries.as_slice() {
            graphs
                .entry(trait_name.clone())
                .or_default()
                .nodes
                .insert(head.clone(), im);
        }
    }

    let mut out = Vec::new();
    for im in impls {
        let Some(trait_name) = adopted_trait(im, routed) else {
            continue;
        };
        let Some(head) = crate::names::local_type_head_ident(&im.self_ty) else {
            continue;
        };
        let head = crate::helper::unraw(&head);
        let Some(graph) = graphs.get(&trait_name) else {
            continue;
        };
        if !graph.nodes.contains_key(&head) {
            continue;
        }
        for (target, ty, path) in graph.opt_out_edges(im, &trait_name, scope) {
            let Some(chain) = graph.chain_back(&target, &head, &trait_name, scope, recurse_level)
            else {
                continue;
            };
            out.push(report(ty, path, &head, &target, &trait_name, &chain));
        }
    }
    out
}

/// One trait's concrete nodes, by self type name.
#[derive(Default)]
struct Graph<'a> {
    nodes: BTreeMap<String, &'a ItemImpl>,
}

/// Which edge a bound is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    /// `B: Loop`: the engine ranks it, one rank down; the floor has none.
    Cycle,
    /// `crate::m::C: Loop`: left as written at every rank; it enters `C`'s chain at the top rank.
    OptOut,
}

impl<'a> Graph<'a> {
    /// The bounds of `im` (a node) naming `trait_name` on another node, with their kind: the
    /// target's name, the bounded type and its path as written.
    fn edges<'i>(
        &self,
        im: &'i ItemImpl,
        trait_name: &str,
        scope: &ModuleScope,
    ) -> Vec<(Edge, String, &'i Type, &'i Path)> {
        let mut out = Vec::new();
        for pt in crate::names::type_predicates(&im.generics) {
            if pt.lifetimes.is_some() || !pt.bounds.iter().any(|b| names_trait(b, trait_name)) {
                continue;
            }
            let Type::Path(TypePath { qself: None, path }) = &pt.bounded_ty else {
                continue;
            };
            let (kind, target) = match bare_name(path) {
                Some(name) => (Edge::Cycle, name),
                None => match scope.own_type_by_path(path) {
                    Some(name) => (Edge::OptOut, name),
                    None => continue,
                },
            };
            if self.nodes.contains_key(&target) {
                out.push((kind, target, &pt.bounded_ty, path));
            }
        }
        out
    }

    fn opt_out_edges<'i>(
        &self,
        im: &'i ItemImpl,
        trait_name: &str,
        scope: &ModuleScope,
    ) -> Vec<(String, &'i Type, &'i Path)> {
        self.edges(im, trait_name, scope)
            .into_iter()
            .filter(|(kind, ..)| *kind == Edge::OptOut)
            .map(|(_, target, ty, path)| (target, ty, path))
            .collect()
    }

    /// The shortest chain of types from `from` (entered through the original trait, at the top
    /// rank) to `to`'s impl, at any rank: `[from, .., to]`, or `None` when the rank chain never
    /// reaches it.
    ///
    /// Searched over `(node, ranks left)` with the crate's graph machinery (`crate::graph`, as for
    /// the obligation graph in `crate::analysis` and `sharing_sources`). The top rank is capped at
    /// the number of nodes: a chain never needs a longer run of cycle edges than that, so the
    /// answer is the same, and the search stays small for a large `recurse_level`.
    fn chain_back(
        &self,
        from: &str,
        to: &str,
        trait_name: &str,
        scope: &ModuleScope,
        recurse_level: usize,
    ) -> Option<Vec<String>> {
        use safegraph::algo::shortest_path::dijkstra;
        use safegraph::graph::Graph as _;
        let top = recurse_level.min(self.nodes.len());
        let states: Vec<(String, usize)> = self
            .nodes
            .keys()
            .flat_map(|name| (0..=top).map(move |rank| (name.clone(), rank)))
            .collect();
        let position: HashMap<(String, usize), usize> = states.iter().cloned().zip(0..).collect();
        let mut edges: Vec<(usize, usize, ())> = Vec::new();
        for (name, im) in &self.nodes {
            for (kind, target, ..) in self.edges(im, trait_name, scope) {
                for rank in 0..=top {
                    let next = match kind {
                        Edge::Cycle if rank == 0 => continue,
                        Edge::Cycle => rank - 1,
                        Edge::OptOut => top,
                    };
                    edges.push((
                        position[&(name.clone(), rank)],
                        position[&(target.clone(), next)],
                        (),
                    ));
                }
            }
        }
        let graph = crate::graph::compose(states, edges);
        graph.scope(|ctx| {
            let ix_of = |name: &str, rank: usize| {
                ctx.node_indices()
                    .map(|ix| ix.to_ix())
                    .find(|ix| *ctx.node(*ix) == (name.to_owned(), rank))
            };
            let start = ix_of(from, top)?;
            let reached = dijkstra(ctx, start, None, |_| 1usize);
            let (_, goal) = (0..=top)
                .filter_map(|rank| ix_of(to, rank))
                .filter_map(|ix| reached.get(&ix).map(|(dist, _)| (*dist, ix)))
                .min_by_key(|(dist, _)| *dist)?;
            let mut chain = vec![ctx.node(goal).0.clone()];
            let mut at = goal;
            while let Some((_, Some(prev))) = reached.get(&at) {
                chain.push(ctx.node(*prev).0.clone());
                at = *prev;
            }
            chain.reverse();
            Some(chain)
        })
    }
}

/// The routed trait an impl the engine takes over implements, by name: a bare (or `self::`) trait
/// path of one segment without arguments, not negative.
fn adopted_trait(im: &ItemImpl, routed: &HashSet<Ident>) -> Option<String> {
    let (bang, path, _) = im.trait_.as_ref()?;
    if bang.is_some() {
        return None;
    }
    let mut path = path.clone();
    strip_leading_self(&mut path);
    let seg = match (path.leading_colon, path.segments.len()) {
        (None, 1) => &path.segments[0],
        _ => return None,
    };
    (seg.arguments.is_none() && routed.contains(&seg.ident))
        .then(|| crate::helper::unraw(&seg.ident))
}

/// Is `im` (an impl of a routed trait for the bare `head`) a concrete node? See the module docs.
fn is_concrete(im: &ItemImpl, head: &Ident, scope: &ModuleScope) -> bool {
    let Type::Path(TypePath { qself: None, path }) = &*im.self_ty else {
        return false;
    };
    im.generics.params.is_empty()
        && im.defaultness.is_none()
        && path.segments.iter().all(|seg| seg.arguments.is_none())
        && scope.declares_nominal(head)
        && im.attrs.iter().all(is_inert)
}

/// An attribute that changes nothing about which items exist, or how: a lint level or a doc
/// comment. Anything else (`#[cfg]`, `#[cfg_attr]`, an attribute macro) may remove or rewrite the
/// item once it expands, after this pass has read it.
pub(crate) fn is_inert(attr: &Attribute) -> bool {
    ["allow", "warn", "deny", "forbid", "expect", "doc"]
        .iter()
        .any(|name| attr.path().is_ident(name))
}

/// Is `bound` the plain bound `trait_name` (bare or `self::`, no arguments, no modifier, no binder)?
fn names_trait(bound: &TypeParamBound, trait_name: &str) -> bool {
    let TypeParamBound::Trait(TraitBound {
        paren_token: None,
        modifier: TraitBoundModifier::None,
        lifetimes: None,
        path,
    }) = bound
    else {
        return false;
    };
    bare_name(path).is_some_and(|name| name == trait_name)
}

/// The name a bare (or `self::`) path of one segment without arguments spells.
fn bare_name(path: &Path) -> Option<String> {
    let mut path = path.clone();
    strip_leading_self(&mut path);
    match (path.leading_colon, path.segments.len()) {
        (None, 1) if path.segments[0].arguments.is_none() => {
            Some(crate::helper::unraw(&path.segments[0].ident))
        }
        _ => None,
    }
}

/// `path` as the user wrote it, without the token spacing (`crate::m::C`).
fn written(path: &Path) -> String {
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    let lead = path.leading_colon.map_or("", |_| "::");
    format!("{lead}{}", segments.join("::"))
}

fn report(
    ty: &Type,
    path: &Path,
    carrier: &str,
    target: &str,
    trait_name: &str,
    chain: &[String],
) -> OptOutCycle {
    let written = written(path);
    let cycle = std::iter::once(carrier)
        .chain(chain.iter().map(String::as_str))
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(" → ");
    let leads_back = if target == carrier {
        format!("it bounds `{target}`'s own impl of `{trait_name}` ({cycle})")
    } else {
        format!("`{target}`'s impl of `{trait_name}` leads back to this impl ({cycle})")
    };
    OptOutCycle {
        at: ty.clone(),
        message: format!(
            "decycle: `{written}` names `{target}` of this module by its path, so this bound is \
             left as written, but {leads_back}: rustc cannot prove that cycle (E0275)"
        ),
        help: format!("write `{target}` to make the bound part of the cycle decycle breaks"),
    }
}

/// `item`, for the fallback emitted after the errors: an impl's `where` predicates that name no
/// generic parameter of the impl (nor `Self`) lose their bounds on the traits in `routed`.
///
/// rustc proves such a global predicate where it is written, so on a cycle (the reported one, or
/// the cycles the engine would have broken) each one is an `E0275` of its own. Without them the
/// impls hold unconditionally, which is all the bodies need; the bounds on generic parameters,
/// which bodies do rely on, stay.
pub(crate) fn quiet(mut item: Item, routed: &HashSet<Ident>) -> Item {
    let Item::Impl(im) = &mut item else {
        return item;
    };
    let mut params: HashSet<String> = HashSet::new();
    let mut lifetimes: HashSet<String> = HashSet::new();
    for param in &im.generics.params {
        match param {
            GenericParam::Type(t) => params.insert(t.ident.to_string()),
            GenericParam::Const(c) => params.insert(c.ident.to_string()),
            GenericParam::Lifetime(l) => lifetimes.insert(l.lifetime.ident.to_string()),
        };
    }
    let Some(where_clause) = &mut im.generics.where_clause else {
        return item;
    };
    let predicates = std::mem::take(&mut where_clause.predicates);
    for mut pred in predicates {
        if let WherePredicate::Type(pt) = &mut pred {
            let global = pt.lifetimes.is_none() && {
                let mut v = MentionsParam {
                    params: &params,
                    lifetimes: &lifetimes,
                    found: false,
                };
                v.visit_predicate_type(pt);
                !v.found
            };
            if global {
                let bounds = std::mem::take(&mut pt.bounds);
                pt.bounds = bounds
                    .into_iter()
                    .filter(|b| !names_routed(b, routed))
                    .collect();
                if pt.bounds.is_empty() {
                    continue;
                }
            }
        }
        where_clause.predicates.push(pred);
    }
    if where_clause.predicates.is_empty() {
        im.generics.where_clause = None;
    }
    item
}

/// Does a bound name a routed trait, by its last segment (any spelling: the fallback drops the
/// path-spelled opt-out of a trait, `crate::m::Loop`, as well)?
fn names_routed(bound: &TypeParamBound, routed: &HashSet<Ident>) -> bool {
    matches!(bound, TypeParamBound::Trait(TraitBound { path, .. })
        if path.segments.last().is_some_and(|seg| routed.contains(&seg.ident)))
}

/// Does a predicate mention one of the impl's generic parameters, or `Self`?
struct MentionsParam<'a> {
    params: &'a HashSet<String>,
    lifetimes: &'a HashSet<String>,
    found: bool,
}

impl Visit<'_> for MentionsParam<'_> {
    fn visit_ident(&mut self, ident: &Ident) {
        if ident == "Self" || self.params.contains(&ident.to_string()) {
            self.found = true;
        }
    }

    fn visit_lifetime(&mut self, lifetime: &syn::Lifetime) {
        if self.lifetimes.contains(&lifetime.ident.to_string()) {
            self.found = true;
        }
    }
}

#[cfg(test)]
mod tests;
