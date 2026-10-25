//! The small directed graphs the passes read a module's cycles off, over `safegraph`.
//!
//! Every graph here is built by [`compose`] from a node list and edges between node **positions**,
//! and read back by position: [`crate::analysis`]'s obligation graph (for `crate::growth`), the
//! structural engine's `(type, trait)` cycles (`Model::cyclic_sccs`), the ranked engine's
//! premise-sharing scope (`sharing_sources`) and its opt-out search (`ranked::opt_out`).
//!
//! `VecGraph` is `Vec`-backed, so its indices are not stable across removals and a node is only
//! inserted inside a scope; nothing is ever removed here, and a node's position is its index in
//! insertion order, in every scope.

use safegraph::graph::Graph;
use safegraph::VecGraph;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;

/// A graph of `nodes`, inserted in the order given, and `edges` between them by node position, in
/// the order given. An edge referring to an absent position is dropped.
pub(crate) fn compose<N: Debug + 'static, E: Debug + 'static>(
    nodes: impl IntoIterator<Item = N>,
    edges: impl IntoIterator<Item = (usize, usize, E)>,
) -> VecGraph<N, E> {
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

/// [`compose`]'s inverse: the nodes in order, and the edges by node position.
pub(crate) fn decompose<N: Clone + 'static, E: Clone + 'static>(
    graph: &VecGraph<N, E>,
) -> (Vec<N>, Vec<(usize, usize, E)>) {
    graph.scope(|ctx| {
        let order: Vec<_> = ctx.node_indices().map(|ix| ix.to_ix()).collect();
        let pos: HashMap<_, usize> = order.iter().enumerate().map(|(i, ix)| (*ix, i)).collect();
        let nodes = order.iter().map(|ix| ctx.node(*ix).clone()).collect();
        let edges = ctx
            .edge_indices()
            .map(|e| {
                let e = e.to_ix();
                let mut ends = ctx.endpoints(e).map(|n| pos[&n.to_ix()]);
                let (a, b) = (ends.next().unwrap(), ends.next().unwrap());
                (a, b, ctx.edge(e).clone())
            })
            .collect();
        (nodes, edges)
    })
}

/// For each node, by position, the positions of the nodes reachable from it, itself included.
pub(crate) fn reachable<N: 'static, E: 'static>(graph: &VecGraph<N, E>) -> Vec<HashSet<usize>> {
    use safegraph::algo::dfs::Dfs;
    graph.scope(|ctx| {
        let order: Vec<_> = ctx.node_indices().map(|ix| ix.to_ix()).collect();
        let pos: HashMap<_, usize> = order.iter().enumerate().map(|(i, ix)| (*ix, i)).collect();
        order
            .iter()
            .map(|ix| Dfs::new(ctx, *ix).map(|n| pos[&n]).collect())
            .collect()
    })
}

/// The strongly connected components that lie on a cycle, by position: those of more than one
/// node, and a node with an edge to itself. A lone node without one is not recursive, however many
/// other nodes point at it. Each component is in ascending order, and the components are ordered by
/// their first node.
pub(crate) fn cyclic_components<N: Clone + 'static, E: Clone + 'static>(
    graph: &VecGraph<N, E>,
) -> Vec<Vec<usize>> {
    let (_, edges) = decompose(graph);
    let self_looped: HashSet<usize> = edges
        .iter()
        .filter(|(a, b, _)| a == b)
        .map(|(a, _, _)| *a)
        .collect();
    let mut out: Vec<Vec<usize>> = graph.scope(|ctx| {
        let order: Vec<_> = ctx.node_indices().map(|ix| ix.to_ix()).collect();
        let pos: HashMap<_, usize> = order.iter().enumerate().map(|(i, ix)| (*ix, i)).collect();
        safegraph::algo::connectivity::tarjan_scc(ctx)
            .map(|component| {
                let mut members: Vec<usize> = component.iter().map(|ix| pos[ix]).collect();
                members.sort_unstable();
                members
            })
            .filter(|members| members.len() > 1 || self_looped.contains(&members[0]))
            .collect()
    });
    out.sort_unstable();
    out
}
