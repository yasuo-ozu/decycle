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
    /// Every cyclic SCC over the `#[decycle]`-annotated traits (`allowed_traits`, keyed by the bare
    /// name, `collect::trait_key`). Impls of other traits are left untouched.
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
        let participates =
            |im: &crate::structural::collect::ImplBlock| allowed.contains(&im.trait_key);
        let mut adj: BTreeMap<Pair, BTreeSet<Pair>> = BTreeMap::new();
        // node: (self type, trait) for every impl of an allowed trait
        for im in &self.impls {
            if participates(im) {
                adj.entry((im.self_ident.to_string(), im.trait_key.clone()))
                    .or_default();
            }
        }
        // edge: `impl Tr for A where <ref B>: TrB`  =>  (A, Tr) -> (B, TrB), when (B, TrB) is a node.
        for im in &self.impls {
            if !participates(im) {
                continue;
            }
            let src: Pair = (im.self_ident.to_string(), im.trait_key.clone());
            for pred in im.item.generics.where_clause.iter().flat_map(|w| &w.predicates) {
                if let WherePredicate::Type(pt) = pred {
                    // Resolve `Self` to the impl's self type so `where Self: Tr` forms a real cycle
                    // edge — `Self` never matches a local ADT ident on its own.
                    let bounded = subst_self(&pt.bounded_ty, &im.item.self_ty);
                    let refs = local_refs(&bounded, adt_names);
                    for bound in &pt.bounds {
                        let syn::TypeParamBound::Trait(tb) = bound else {
                            continue;
                        };
                        let Some(btr) = trait_key(&tb.path) else {
                            continue;
                        };
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
///
/// A local ADT is named by its bare (or `self::`-qualified) name, as in the ranked engine: a path
/// the module writes (`crate::m::Stmt`, `super::Stmt`) names the item itself, the opt-out, and is no
/// edge of the cycle. The arguments of any path are still visited, so `crate::other::Wrap<Stmt>`
/// names `Stmt`.
pub(crate) fn local_refs(ty: &Type, adt_names: &HashSet<String>) -> Vec<String> {
    struct Refs<'a> {
        adt_names: &'a HashSet<String>,
        out: Vec<String>,
    }
    impl<'ast> syn::visit::Visit<'ast> for Refs<'_> {
        fn visit_type_path(&mut self, tp: &'ast syn::TypePath) {
            if tp.qself.is_none() {
                if let Some(ident) = bare_ident(&tp.path) {
                    let s = ident.to_string();
                    if self.adt_names.contains(&s) && !self.out.contains(&s) {
                        self.out.push(s);
                    }
                }
            }
            syn::visit::visit_type_path(self, tp);
        }
    }
    let mut refs = Refs {
        adt_names,
        out: Vec::new(),
    };
    syn::visit::Visit::visit_type(&mut refs, ty);
    refs.out
}

/// Visit every path-segment ident within `ty`, descending references/containers/tuples/generic args,
/// and also into fn-pointer, `dyn` and `impl Trait` types (so a `Self` inside `fn(&Self)` /
/// `dyn Fn(&Self)` / `impl Fn(&Self)` is seen).
pub(crate) fn walk_type(ty: &Type, f: &mut impl FnMut(&Ident)) {
    match ty {
        Type::Path(tp) => {
            if let Some(q) = &tp.qself {
                walk_type(&q.ty, f);
            }
            if let Some(seg) = tp.path.segments.last() {
                f(&seg.ident);
                walk_args(&seg.arguments, f);
            }
        }
        Type::Reference(r) => walk_type(&r.elem, f),
        Type::Ptr(p) => walk_type(&p.elem, f),
        Type::Array(a) => walk_type(&a.elem, f),
        Type::Slice(s) => walk_type(&s.elem, f),
        Type::Tuple(t) => t.elems.iter().for_each(|e| walk_type(e, f)),
        Type::Paren(p) => walk_type(&p.elem, f),
        Type::Group(g) => walk_type(&g.elem, f),
        Type::BareFn(bf) => {
            for input in &bf.inputs {
                walk_type(&input.ty, f);
            }
            if let syn::ReturnType::Type(_, t) = &bf.output {
                walk_type(t, f);
            }
        }
        Type::TraitObject(to) => walk_bounds(&to.bounds, f),
        Type::ImplTrait(it) => walk_bounds(&it.bounds, f),
        _ => {}
    }
}

/// Visit types inside a path segment's arguments — both `<..>` and `Fn(..)`-sugar forms.
fn walk_args(args: &PathArguments, f: &mut impl FnMut(&Ident)) {
    match args {
        PathArguments::AngleBracketed(ab) => {
            for a in &ab.args {
                if let GenericArgument::Type(t) = a {
                    walk_type(t, f);
                }
            }
        }
        PathArguments::Parenthesized(p) => {
            p.inputs.iter().for_each(|t| walk_type(t, f));
            if let syn::ReturnType::Type(_, t) = &p.output {
                walk_type(t, f);
            }
        }
        PathArguments::None => {}
    }
}

/// Visit types inside the trait bounds of a `dyn`/`impl Trait` type.
fn walk_bounds(
    bounds: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    f: &mut impl FnMut(&Ident),
) {
    for b in bounds {
        if let syn::TypeParamBound::Trait(tb) = b {
            if let Some(seg) = tb.path.segments.last() {
                f(&seg.ident);
                walk_args(&seg.arguments, f);
            }
        }
    }
}
