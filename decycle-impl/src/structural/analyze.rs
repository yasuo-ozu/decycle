//! Analysis: an obligation graph over `(type, trait)` PAIRS → cyclic SCCs. Keying on pairs (not just
//! types) is what lets a **cross-trait** cycle — `impl Eval for Expr where Expr: Size` +
//! `impl Size for Expr where Expr: Eval` — be recognised: the nodes `(Expr, Eval)` and `(Expr, Size)`
//! form one SCC even though no single trait's sub-graph is cyclic.

use super::*;

/// A `(type_ident, trait_key)` node in the obligation graph.
pub(crate) type Pair = (String, String);

/// A recursion cycle: an SCC of the obligation graph, i.e. a set of mutually-cyclic `(type, trait)`
/// pairs. Each distinct *type* in it gets one `#[repr(transparent)]` terminator; each *pair* gets a
/// terminator+natural trait impl.
pub(crate) struct Scc {
    pub pairs: Vec<Pair>,
}

impl Scc {
    pub fn contains(&self, ty: &str, tr: &str) -> bool {
        self.pairs.iter().any(|(t, k)| t == ty && k == tr)
    }
    /// The distinct cycle-member type idents (each gets one terminator).
    pub fn types(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for (t, _) in &self.pairs {
            if !v.contains(t) {
                v.push(t.clone());
            }
        }
        v
    }
}

/// A directed graph over `(type, trait)` pairs; edges from `where`-clause references.
struct Graph {
    adj: BTreeMap<Pair, BTreeSet<Pair>>,
}

impl Model {
    /// Every cyclic SCC over the `#[decycle]`-annotated traits (`allowed_traits`, keyed by
    /// last-segment ident). Impls of other traits are left untouched.
    pub fn cyclic_sccs(&self, allowed_traits: &HashSet<String>) -> Vec<Scc> {
        let adt_names: HashSet<String> = self.adts.keys().cloned().collect();
        let graph = self.build_graph(allowed_traits, &adt_names);
        let mut out = Vec::new();
        for comp in graph.sccs() {
            if !graph.is_cyclic_component(&comp) {
                continue; // a lone pair with no self-edge is not a recursion cycle
            }
            out.push(Scc { pairs: comp });
        }
        out
    }

    fn build_graph(&self, allowed: &HashSet<String>, adt_names: &HashSet<String>) -> Graph {
        let mut adj: BTreeMap<Pair, BTreeSet<Pair>> = BTreeMap::new();
        // node: (self type, trait) for every impl of an allowed trait
        for im in &self.impls {
            if allowed.contains(&im.trait_key) {
                adj.entry((im.self_ident.to_string(), im.trait_key.clone()))
                    .or_default();
            }
        }
        // edge: `impl Tr for A where <ref B>: TrB`  =>  (A, Tr) -> (B, TrB), when (B, TrB) is a node.
        for im in &self.impls {
            if !allowed.contains(&im.trait_key) {
                continue;
            }
            let src: Pair = (im.self_ident.to_string(), im.trait_key.clone());
            for pred in im.item.generics.where_clause.iter().flat_map(|w| &w.predicates) {
                if let WherePredicate::Type(pt) = pred {
                    let refs = local_refs(&pt.bounded_ty, adt_names);
                    for bound in &pt.bounds {
                        let syn::TypeParamBound::Trait(tb) = bound else {
                            continue;
                        };
                        let Some(seg) = tb.path.segments.last() else {
                            continue;
                        };
                        let btr = seg.ident.to_string();
                        if !allowed.contains(&btr) {
                            continue;
                        }
                        for b in &refs {
                            let dst: Pair = (b.clone(), btr.clone());
                            if adj.contains_key(&dst) {
                                adj.get_mut(&src).unwrap().insert(dst);
                            }
                        }
                    }
                }
            }
        }
        Graph { adj }
    }
}

impl Graph {
    fn nodes(&self) -> Vec<Pair> {
        self.adj.keys().cloned().collect()
    }

    /// Tarjan-free SCC via mutual reachability (node counts are tiny). Two nodes share a component iff
    /// mutually reachable; a node is always in its own.
    fn sccs(&self) -> Vec<Vec<Pair>> {
        let nodes = self.nodes();
        let mut seen: HashSet<Pair> = HashSet::new();
        let mut comps = Vec::new();
        for n in &nodes {
            if seen.contains(n) {
                continue;
            }
            let comp: Vec<Pair> = nodes
                .iter()
                .filter(|m| *m == n || (self.reaches(n, m) && self.reaches(m, n)))
                .cloned()
                .collect();
            for m in &comp {
                seen.insert(m.clone());
            }
            comps.push(comp);
        }
        comps
    }

    fn reaches(&self, from: &Pair, to: &Pair) -> bool {
        if from == to {
            // reflexive only through a real path (self-edge)
            return self.adj.get(from).is_some_and(|s| s.contains(to));
        }
        let mut stack = vec![from.clone()];
        let mut seen = HashSet::new();
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            if let Some(succ) = self.adj.get(&cur) {
                if succ.contains(to) {
                    return true;
                }
                for s in succ {
                    stack.push(s.clone());
                }
            }
        }
        false
    }

    /// Is this component an actual cycle (size > 1, or a lone self-looping pair)?
    fn is_cyclic_component(&self, comp: &[Pair]) -> bool {
        if comp.len() > 1 {
            return true;
        }
        let n = &comp[0];
        self.adj.get(n).is_some_and(|s| s.contains(n))
    }
}

/// Local ADT idents appearing anywhere in `ty` (peeling references/containers/tuples).
pub(crate) fn local_refs(ty: &Type, adt_names: &HashSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    walk_type(ty, &mut |id| {
        let s = id.to_string();
        if adt_names.contains(&s) && !out.contains(&s) {
            out.push(s);
        }
    });
    out
}

/// Visit every path-segment ident within `ty`, descending references/containers/tuples/generic args.
pub(crate) fn walk_type(ty: &Type, f: &mut impl FnMut(&Ident)) {
    match ty {
        Type::Path(tp) => {
            if let Some(q) = &tp.qself {
                walk_type(&q.ty, f);
            }
            if let Some(seg) = tp.path.segments.last() {
                f(&seg.ident);
                if let PathArguments::AngleBracketed(ab) = &seg.arguments {
                    for a in &ab.args {
                        if let GenericArgument::Type(t) = a {
                            walk_type(t, f);
                        }
                    }
                }
            }
        }
        Type::Reference(r) => walk_type(&r.elem, f),
        Type::Ptr(p) => walk_type(&p.elem, f),
        Type::Array(a) => walk_type(&a.elem, f),
        Type::Slice(s) => walk_type(&s.elem, f),
        Type::Tuple(t) => t.elems.iter().for_each(|e| walk_type(e, f)),
        Type::Paren(p) => walk_type(&p.elem, f),
        Type::Group(g) => walk_type(&g.elem, f),
        _ => {}
    }
}
