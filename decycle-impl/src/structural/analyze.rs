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
    pub fn types(&self) -> HashSet<String> {
        self.pairs.iter().map(|(t, _)| t.clone()).collect()
    }
}

impl Model {
    /// Every cyclic SCC over the `#[decycle]`-annotated traits (`allowed_traits`, keyed by the bare
    /// name, `collect::trait_key`): the components of the graph over `(type, trait)` pairs that lie
    /// on a cycle (more than one pair, or a lone self-looping pair), each pair-sorted, in the order
    /// of their smallest pair. Impls of other traits are left untouched.
    pub fn cyclic_sccs(&self, allowed_traits: &HashSet<String>) -> Vec<Scc> {
        let adt_names: HashSet<String> = self.adts.keys().cloned().collect();
        let participating = || {
            self.impls
                .iter()
                .filter(move |im| allowed_traits.contains(&im.trait_key))
        };
        let pair = |im: &ImplBlock| -> Pair { (im.self_ident.to_string(), im.trait_key.clone()) };
        // node: (self type, trait) for every impl of an allowed trait
        let nodes: Vec<Pair> = participating()
            .map(pair)
            .collect::<BTreeSet<Pair>>()
            .into_iter()
            .collect();
        let position = |p: &Pair| nodes.binary_search(p).ok();
        // edge: `impl Tr for A where <ref B>: TrB`  =>  (A, Tr) -> (B, TrB), when (B, TrB) is a node.
        let mut edges = Vec::new();
        for im in participating() {
            let src = position(&pair(im)).expect("every participating impl is a node");
            for pt in crate::names::type_predicates(&im.item.generics) {
                // Resolve `Self` to the impl's self type so `where Self: Tr` forms a real cycle
                // edge — `Self` never matches a local ADT ident on its own.
                let bounded = subst_self(&pt.bounded_ty, &im.item.self_ty);
                let refs = local_refs(&bounded, &adt_names);
                for bound in &pt.bounds {
                    let syn::TypeParamBound::Trait(tb) = bound else {
                        continue;
                    };
                    let Some(btr) = trait_key(&tb.path).filter(|t| allowed_traits.contains(t))
                    else {
                        continue;
                    };
                    for b in &refs {
                        if let Some(dst) = position(&(b.clone(), btr.clone())) {
                            edges.push((src, dst, ()));
                        }
                    }
                }
            }
        }
        let graph = crate::graph::compose(nodes.iter().cloned(), edges);
        crate::graph::cyclic_components(&graph)
            .into_iter()
            .map(|component| Scc {
                pairs: component.into_iter().map(|k| nodes[k].clone()).collect(),
            })
            .collect()
    }
}

/// Local ADT idents appearing anywhere in `ty` (peeling references/containers/tuples), outermost
/// first, without repeats.
///
/// A local ADT is named by its bare (or `self::`-qualified) name, as in the ranked engine: a path
/// the module writes (`crate::m::Stmt`, `super::Stmt`) names the item itself, the opt-out, and is no
/// edge of the cycle. The arguments of any path are still visited, so `crate::other::Wrap<Stmt>`
/// names `Stmt`. See [`crate::names::BareNamed`].
pub(crate) fn local_refs(ty: &Type, adt_names: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tp in local_ref_paths(ty, adt_names) {
        let name = bare_ident(&tp.path).map(|i| i.to_string());
        if let Some(name) = name.filter(|name| !out.contains(name)) {
            out.push(name);
        }
    }
    out
}

/// The occurrences of local ADTs in `ty`, as [`local_refs`] finds them: each path naming one, with
/// its arguments (`Term<S>` in `Box<Term<S>>`).
pub(crate) fn local_ref_paths(ty: &Type, adt_names: &HashSet<String>) -> Vec<syn::TypePath> {
    crate::names::bare_named_in(ty, &|i| adt_names.contains(&i.to_string()))
        .into_iter()
        .cloned()
        .collect()
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
