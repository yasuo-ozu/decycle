//! Calls through a wrapper type whose parts are partly outside the cycle.
//!
//! `impl Loop for A where (B, crate::other::Leaf): Loop`, with the pair's own impl taken over
//! (`impl<X: Loop, Y: Loop> Loop for (X, Y)`), used to break in two ways. `peel` replaced the bound
//! by its parts (`B: Loop`, `crate::other::Leaf: Loop`), so nothing fixed the rank of a call through
//! the pair; and every ranked copy of the pair above the floor demands `Leaf: LoopRanked<R>`, which
//! a type outside the cycle never has. The floor copy was the only one that applied, so the first
//! call through the pair landed on it, and no frame had registered the pair's re-entry fn: a run-time
//! "re-entry fn not registered" panic (or E0277 for `<(B, Leaf) as Loop>::step`, which goes through
//! the pair's delegating impl and needs the whole chain).
//!
//! Two things fix it, both decided here:
//!
//! * **The wrapper stays ranked.** `peel` keeps `(B, crate::other::Leaf): Loop` next to its parts
//!   when an impl taken over implements the wrapper's outer shape. It is an edge like any other
//!   (`helper::contains_bare_member`): rank-lowered in the inductive copy, so a call through the
//!   pair resolves at the frame's rank, and registered for the floor.
//! * **Bridges.** A type the wrapper's ranked chain reaches that no impl taken over implements, and
//!   that contains no cycle member, gets a ranked impl that delegates every item to the real trait:
//!   `impl<R> LoopRanked<R> for crate::other::Leaf { fn step(&self, n: u32) -> u32 {
//!   <Self as Loop>::step(self, n) } }`. It is a type outside the cycle, so it needs no rank.
//!
//! A chain is planned structurally against the impls taken over ([`Planner`]); whatever cannot be
//! established syntactically fails the plan, and a failed plan changes nothing: the kept wrapper
//! bound is dropped again ([`settle`]), so the impl is exactly what it was before. In particular a
//! bridge is emitted only when no impl of the ranked trait could overlap it — not for a type an impl
//! taken over might implement, not for a type spelled by a relative path, and not for two spellings
//! that may name one type — because a bridge that overlaps is E0119 on code that compiled before.

use super::*;
use std::collections::HashSet;
use syn::visit_mut::VisitMut;

// `finalize`'s `parse_quote!` (template-quote interpolation), under a name the glob import of `syn`
// does not also bring in.
macro_rules! pq {
    ($($tt:tt)*) => {
        syn::parse2(::template_quote::quote!($($tt)*)).unwrap()
    };
}

/// Walks deeper than this are given up on (the plan fails). The cyclic-bound graphs of the suite need
/// a handful of entries; a non-regular bound never closes.
const PLAN_CAP: usize = 256;

/// One type a planned chain reaches, under one trait.
pub(super) struct Entry {
    pub trait_ident: Ident,
    pub ty: Type,
    pub targs: Vec<GenericArgument>,
    pub kind: Kind,
}

pub(super) enum Kind {
    /// An impl taken over implements it — `impls`, by index among its trait's; `children` are the
    /// entries their ranked copies demand.
    Node {
        children: Vec<usize>,
        impls: Vec<usize>,
    },
    /// No impl taken over implements it: it gets a bridge.
    Bridge,
    /// `Self` or a bare parameter of the planning impl: the caller's to prove, as it always was.
    Param,
}

/// A bridge one plan needs: `impl<..> TrRanked<.., R, ..> for ty`, delegating to `Tr`.
#[derive(Clone)]
pub(super) struct BridgeReq {
    pub trait_ident: Ident,
    /// The type, with the planning impl's parameters renamed to the bridge's own.
    pub ty: Type,
    /// The bridge's own parameters (lifetimes first), declared as `params`.
    pub params: Vec<GenericParam>,
    /// The `#[cfg]`s of the impl that needs it.
    pub cfgs: Vec<Attribute>,
    /// `trait|type` with the bridge's parameters renamed positionally: equal keys are one bridge.
    pub key: String,
    /// The parameter idents of `ty`, for the overlap check between two bridges.
    pub vars: HashSet<Ident>,
}

/// A planned chain: `entries[0]` is the bound it was planned for.
pub(super) struct Plan {
    pub entries: Vec<Entry>,
    pub bridges: Vec<BridgeReq>,
}

fn is_bare_param(ty: &Type, params: &HashSet<Ident>) -> bool {
    matches!(ty, Type::Path(TypePath { qself: None, path })
        if path.leading_colon.is_none()
            && path.segments.len() == 1
            && path.segments[0].arguments == PathArguments::None
            && params.contains(&path.segments[0].ident))
}

/// Does `ty` mention one of `idents` as a single-segment path?
fn mentions(ty: &Type, idents: &HashSet<Ident>) -> bool {
    struct V<'a>(&'a HashSet<Ident>, bool);
    impl<'ast> syn::visit::Visit<'ast> for V<'_> {
        fn visit_path(&mut self, p: &'ast Path) {
            if p.leading_colon.is_none()
                && p.segments.len() == 1
                && self.0.contains(&p.segments[0].ident)
            {
                self.1 = true;
            }
            syn::visit::visit_path(self, p);
        }
    }
    let mut v = V(idents, false);
    syn::visit::Visit::visit_type(&mut v, ty);
    v.1
}

fn mentions_self(ty: &Type) -> bool {
    let mut s = HashSet::new();
    s.insert(Ident::new("Self", Span::call_site()));
    mentions(ty, &s)
}

/// The bare routed-trait bounds of `im` that its ranked copies rank-lower — the ones
/// `remove_cyclic_bounds` strips: `(target, trait, args, has_binder)`, inline bounds included.
fn cyclic_requirements(
    im: &ItemImpl,
    table: &Table,
    heads: &HashSet<Ident>,
) -> Vec<(Type, Ident, Vec<GenericArgument>, bool)> {
    let params = crate::names::type_param_idents(&im.generics);
    let mut out = Vec::new();
    let mut push = |target: &Type, b: &TypeParamBound, binder: bool| {
        let TypeParamBound::Trait(tb) = b else { return };
        let mut path = tb.path.clone();
        crate::names::strip_leading_self(&mut path);
        if path.leading_colon.is_some() || path.segments.len() != 1 {
            return;
        }
        let seg = &path.segments[0];
        if !table.contains_key(&seg.ident) {
            return;
        }
        let participant = is_bare_param(target, &params)
            || is_self_type(target)
            || crate::helper::type_head_ident(target).is_some_and(|h| heads.contains(&h))
            || crate::names::names_bare(target, heads);
        if participant {
            out.push((
                target.clone(),
                seg.ident.clone(),
                nonlifetime_path_args(&seg.arguments),
                binder || tb.lifetimes.is_some(),
            ));
        }
    };
    for p in &im.generics.params {
        if let GenericParam::Type(tp) = p {
            let target: Type = pq!(#{&tp.ident});
            for b in &tp.bounds {
                push(&target, b, false);
            }
        }
    }
    if let Some(wc) = &im.generics.where_clause {
        for pred in &wc.predicates {
            if let WherePredicate::Type(pt) = pred {
                for b in &pt.bounds {
                    push(&pt.bounded_ty, b, pt.lifetimes.is_some());
                }
            }
        }
    }
    out
}

/// The type params of `im` whose INLINE cyclic bound the floor copy keeps on the real trait: those an
/// associated type or const of the impl mentions (`type Out = T::Out;` is copied verbatim onto the
/// floor, and resolves through `T: Tr`). See [`floor_generics`].
fn floor_kept_params(im: &ItemImpl) -> HashSet<Ident> {
    let params = crate::names::type_param_idents(&im.generics);
    let mut out = HashSet::new();
    for item in &im.items {
        if matches!(item, ImplItem::Fn(_)) {
            continue;
        }
        let tokens = quote!(#item).to_string();
        for p in &params {
            if tokens
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|t| p == t)
            {
                out.insert(p.clone());
            }
        }
    }
    out
}

/// The generics of `im`'s floor copy (`impl<..> TrRanked<()> for SelfTy`): its where-clause without
/// the cyclic bounds, as always; its inline bounds as written — or, when `ranked` (the impl is a
/// wrapper a planned chain goes through, [`Settled::ranked_floor`]), with each INLINE cyclic bound
/// ranked at the floor: `impl<X: Tr, Y: Tr> Tr for (X, Y)` has the floor `impl<X: TrRanked<()>,
/// Y: TrRanked<()>> TrRanked<()> for (X, Y)` — but for a parameter an associated item mentions
/// ([`floor_kept_params`]), whose bound stays on the real trait.
///
/// The floor keeps the inline bound on the REAL trait otherwise. For a cycle member that goes
/// through its delegating impl and the whole rank chain again, and once that chain reaches the
/// wrapper from inside — which a planned chain does — it closes an inductive cycle: E0275 (the
/// ranked engine's long-standing "overflow through a participating container", which this also
/// fixes). Ranked at `()`, the bound still keeps the floor from applying where only
/// `X: TrRanked<Rank>` is known (a call through `(T, U)` in a generic frame resolves at the frame's
/// rank, as it did), and it is discharged by the floors of the parts — or their bridges — without
/// leaving the rank chain. It does no longer accept a part with no ranked impl and no bridge, which
/// only an unranked call that reached the floor at once ever used (and whose floor no frame
/// registered), so that stays as it was where no chain is planned.
pub(super) fn floor_generics(im: &ItemImpl, table: &Table, ranked: bool) -> Generics {
    if !ranked {
        let mut g = im.generics.clone();
        g.where_clause = remove_cyclic_bounds(&im.generics, table, false).where_clause;
        return g;
    }
    let kept = floor_kept_params(im);
    let replacer = TraitReplacer {
        table: table
            .iter()
            .map(|(ident, (_, rank_loc, _))| {
                (ident.clone(), (*rank_loc, pq!(#{name!("{}Ranked", ident)})))
            })
            .collect(),
        rank_type: pq!(()),
    };
    let mut g = im.generics.clone();
    g.where_clause = remove_cyclic_bounds(&im.generics, table, false).where_clause;
    for p in g.params.iter_mut() {
        let GenericParam::Type(tp) = p else { continue };
        if kept.contains(&tp.ident) {
            continue;
        }
        for b in tp.bounds.iter_mut() {
            let TypeParamBound::Trait(tb) = b else {
                continue;
            };
            if !matches!(tb.modifier, TraitBoundModifier::None) {
                continue;
            }
            let mut probe = tb.path.clone();
            crate::names::strip_leading_self(&mut probe);
            if probe.leading_colon.is_none()
                && probe.segments.len() == 1
                && table.contains_key(&probe.segments[0].ident)
            {
                tb.path = probe;
                replacer.try_replace_path(&mut tb.path);
            }
        }
    }
    g
}

/// Does the floor copy of `im` keep an inline cyclic bound on the REAL trait ([`floor_generics`])?
/// A chain through such a wrapper could close the inductive cycle the ranked floor bound avoids.
fn floor_keeps_cyclic_bound(im: &ItemImpl, table: &Table) -> bool {
    let kept = floor_kept_params(im);
    im.generics.params.iter().any(|p| match p {
        GenericParam::Type(tp) if kept.contains(&tp.ident) => tp.bounds.iter().any(|b| {
            let TypeParamBound::Trait(tb) = b else {
                return false;
            };
            let mut probe = tb.path.clone();
            crate::names::strip_leading_self(&mut probe);
            probe.leading_colon.is_none()
                && probe.segments.len() == 1
                && table.contains_key(&probe.segments[0].ident)
        }),
        _ => false,
    })
}

/// The ranked supertraits of `trait_` ([`ranked_supertrait`]) with their arguments for the
/// instantiation `targs` of `trait_`: what a ranked impl of `trait_` at `targs` also demands.
fn ranked_supers(
    trait_: &ItemTrait,
    targs: &[GenericArgument],
    table: &Table,
) -> Vec<(Ident, Vec<GenericArgument>)> {
    let tparams: Vec<&GenericParam> = trait_
        .generics
        .params
        .iter()
        .filter(|p| !matches!(p, GenericParam::Lifetime(_)))
        .collect();
    let mut subst = Subst::new();
    for (p, a) in tparams.iter().zip(targs.iter()) {
        if let (GenericParam::Type(tp), GenericArgument::Type(t)) = (p, a) {
            subst.insert(tp.ident.to_string(), crate::unify::Value::Type(t.clone()));
        }
    }
    let mut out = Vec::new();
    for b in &trait_.supertraits {
        let TypeParamBound::Trait(tb) = b else {
            continue;
        };
        if let Some(sup) = ranked_supertrait(&tb.path, trait_, table) {
            let args = nonlifetime_path_args(&tb.path.segments.last().unwrap().arguments);
            out.push((sup, apply_targs_subst(&args, &subst)));
        }
    }
    out
}

/// Might `a` (with `avars` free) and `b` (with `bvars` free) be the same type? Conservative: a path
/// is matched by its last segment and arguments only (`crate::other::Leaf` may be the `Leaf` a
/// `use` imports), and anything that is not plainly a different kind of type may be anything.
fn may_unify(a: &Type, avars: &HashSet<Ident>, b: &Type, bvars: &HashSet<Ident>) -> bool {
    let a = crate::names::strip_parens(a);
    let b = crate::names::strip_parens(b);
    if is_bare_param(a, avars) || is_bare_param(b, bvars) {
        return true;
    }
    match (a, b) {
        (
            Type::Path(TypePath {
                qself: None,
                path: pa,
            }),
            Type::Path(TypePath {
                qself: None,
                path: pb,
            }),
        ) => {
            let (Some(la), Some(lb)) = (pa.segments.last(), pb.segments.last()) else {
                return true;
            };
            if !ident_may_equal(&la.ident, &lb.ident) {
                return false;
            }
            match (&la.arguments, &lb.arguments) {
                (PathArguments::AngleBracketed(x), PathArguments::AngleBracketed(y)) => {
                    let xt: Vec<&Type> = x
                        .args
                        .iter()
                        .filter_map(|g| match g {
                            GenericArgument::Type(t) => Some(t),
                            _ => None,
                        })
                        .collect();
                    let yt: Vec<&Type> = y
                        .args
                        .iter()
                        .filter_map(|g| match g {
                            GenericArgument::Type(t) => Some(t),
                            _ => None,
                        })
                        .collect();
                    xt.len() != yt.len()
                        || xt
                            .iter()
                            .zip(yt.iter())
                            .all(|(s, t)| may_unify(s, avars, t, bvars))
                }
                _ => true,
            }
        }
        (Type::Tuple(x), Type::Tuple(y)) => {
            x.elems.len() == y.elems.len()
                && x.elems
                    .iter()
                    .zip(y.elems.iter())
                    .all(|(s, t)| may_unify(s, avars, t, bvars))
        }
        (Type::Reference(x), Type::Reference(y)) => {
            x.mutability.is_some() == y.mutability.is_some()
                && may_unify(&x.elem, avars, &y.elem, bvars)
        }
        (Type::Ptr(x), Type::Ptr(y)) => {
            x.mutability.is_some() == y.mutability.is_some()
                && may_unify(&x.elem, avars, &y.elem, bvars)
        }
        (Type::Array(x), Type::Array(y)) => may_unify(&x.elem, avars, &y.elem, bvars),
        (Type::Slice(x), Type::Slice(y)) => may_unify(&x.elem, avars, &y.elem, bvars),
        (Type::Path(_), Type::Path(_)) => true, // a projection on either side
        (Type::Path(TypePath { qself: Some(_), .. }), _)
        | (_, Type::Path(TypePath { qself: Some(_), .. })) => true,
        (Type::Tuple(_), _)
        | (Type::Reference(_), _)
        | (Type::Ptr(_), _)
        | (Type::Array(_), _)
        | (Type::Slice(_), _)
        | (Type::Path(_), _)
            if is_plain_kind(b) =>
        {
            false
        }
        _ => true,
    }
}

/// Might two last path segments name one type? Equal idents may; and `nesting` re-spells a cycle
/// member defined in the module as `__DecycleNat_<name>_<module>`, which may be the item another
/// path names `<name>` (`crate::m::C` against the member `C`) — matched by prefix, conservatively.
fn ident_may_equal(a: &Ident, b: &Ident) -> bool {
    if a == b {
        return true;
    }
    let (a, b) = (crate::helper::unraw(a), crate::helper::unraw(b));
    let aliases = |alias: &str, name: &str| alias.starts_with(&format!("__DecycleNat_{}_", name));
    aliases(&a, &b) || aliases(&b, &a)
}

/// A kind [`may_unify`] tells apart from the other plain kinds.
fn is_plain_kind(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Path(TypePath { qself: None, .. })
            | Type::Tuple(_)
            | Type::Reference(_)
            | Type::Ptr(_)
            | Type::Array(_)
            | Type::Slice(_)
    )
}

/// Can a bridge be written for `ty` at all: a type made only of plain paths, tuples, references,
/// pointers, arrays, slices and trait objects, with no projection, no `Self`, no `_`, no macro, and
/// no lifted relative path (`super::x::Leaf` is re-spelled through a `__DecycleRelMod_*` alias by
/// `nesting`, whose target cannot be compared with anything).
fn bridgeable_shape(ty: &Type) -> bool {
    struct V(bool);
    impl<'ast> syn::visit::Visit<'ast> for V {
        fn visit_type(&mut self, ty: &'ast Type) {
            match ty {
                Type::Path(TypePath { qself: Some(_), .. })
                | Type::ImplTrait(_)
                | Type::Infer(_)
                | Type::Macro(_)
                | Type::Verbatim(_)
                | Type::Never(_) => self.0 = false,
                _ => syn::visit::visit_type(self, ty),
            }
        }
        fn visit_path(&mut self, p: &'ast Path) {
            if let Some(first) = p.segments.first() {
                let s = first.ident.to_string();
                if first.ident == "Self"
                    || first.ident == "self"
                    || first.ident == "super"
                    || s.starts_with("__DecycleRelMod_")
                {
                    self.0 = false;
                }
            }
            syn::visit::visit_path(self, p);
        }
        fn visit_expr(&mut self, _: &'ast Expr) {
            // An array length: fine as written.
        }
    }
    let mut v = V(true);
    syn::visit::Visit::visit_type(&mut v, ty);
    v.0
}

/// The lifetimes, and the type and const params of `generics`, that `ty` mentions, in order of
/// appearance.
fn mentioned_params(ty: &Type, generics: &Generics) -> (Vec<Lifetime>, Vec<GenericParam>) {
    struct V {
        lts: Vec<Lifetime>,
        idents: Vec<Ident>,
    }
    impl<'ast> syn::visit::Visit<'ast> for V {
        fn visit_lifetime(&mut self, l: &'ast Lifetime) {
            if l.ident != "static" && l.ident != "_" && !self.lts.iter().any(|x| x.ident == l.ident)
            {
                self.lts.push(l.clone());
            }
        }
        fn visit_path(&mut self, p: &'ast Path) {
            if p.leading_colon.is_none()
                && p.segments.len() == 1
                && !self.idents.contains(&p.segments[0].ident)
            {
                self.idents.push(p.segments[0].ident.clone());
            }
            syn::visit::visit_path(self, p);
        }
    }
    let mut v = V {
        lts: Vec::new(),
        idents: Vec::new(),
    };
    syn::visit::Visit::visit_type(&mut v, ty);
    let params = v
        .idents
        .iter()
        .filter_map(|i| {
            generics.params.iter().find(|p| match p {
                GenericParam::Type(t) => &t.ident == i,
                GenericParam::Const(c) => &c.ident == i,
                GenericParam::Lifetime(_) => false,
            })
        })
        .cloned()
        .collect();
    (v.lts, params)
}

/// Plans the ranked chains one impl's bounds reach.
pub(super) struct Planner<'a> {
    table: &'a Table,
    heads: HashSet<Ident>,
    impl_: &'a ItemImpl,
    impl_params: HashSet<Ident>,
    visited: HashMap<String, usize>,
    entries: Vec<Entry>,
    bridges: Vec<BridgeReq>,
}

impl<'a> Planner<'a> {
    /// Plan `ty: trait_ident<targs>`, a bound of `impl_`.
    pub(super) fn plan(
        table: &'a Table,
        impl_: &'a ItemImpl,
        ty: &Type,
        trait_ident: &Ident,
        targs: &[GenericArgument],
    ) -> Option<Plan> {
        let mut p = Planner {
            table,
            heads: cycle_self_heads(table),
            impl_,
            impl_params: crate::names::type_param_idents(&impl_.generics),
            visited: HashMap::new(),
            entries: Vec::new(),
            bridges: Vec::new(),
        };
        p.walk(ty, trait_ident, targs)?;
        Some(Plan {
            entries: p.entries,
            bridges: p.bridges,
        })
    }

    fn walk(&mut self, ty: &Type, trait_ident: &Ident, targs: &[GenericArgument]) -> Option<usize> {
        let key = format!("{}|{}|{}", trait_ident, quote!(#ty), quote!(#(#targs),*));
        if let Some(&ix) = self.visited.get(&key) {
            return Some(ix);
        }
        if self.visited.len() >= PLAN_CAP {
            return None;
        }
        let table = self.table;
        let (trait_, _, impls) = table.get(trait_ident)?;
        let ix = self.entries.len();
        self.visited.insert(key, ix);
        self.entries.push(Entry {
            trait_ident: trait_ident.clone(),
            ty: ty.clone(),
            targs: targs.to_vec(),
            kind: Kind::Param,
        });
        if is_bare_param(ty, &self.impl_params) || is_self_type(ty) {
            return Some(ix);
        }
        let mut children = Vec::new();
        let mut matched = Vec::new();
        for (ci, cand) in impls.iter().enumerate() {
            let vars = crate::names::type_param_idents(&cand.generics);
            let cand_targs = impl_trait_args(cand);
            let Some(subst) =
                crate::unify::unify_obligation(&vars, &cand.self_ty, &cand_targs, ty, targs)
            else {
                continue;
            };
            if vars.iter().any(|v| !subst.contains_key(&v.to_string())) {
                return None;
            }
            // The floor copy of a wrapper that keeps a real cyclic bound would close an inductive
            // cycle through the delegating impls (see `floor_generics`).
            if floor_keeps_cyclic_bound(cand, table) {
                return None;
            }
            matched.push(ci);
            for (target, tr, ta, binder) in cyclic_requirements(cand, table, &self.heads) {
                // A requirement that names none of the impl's parameters is the cycle's own business:
                // it reads the same at every instantiation, and the impl's own frames rank it.
                if !mentions(&target, &vars) && !mentions_self(&target) {
                    continue;
                }
                if binder {
                    return None;
                }
                let target = crate::unify::substitute(
                    &subst_bare_self_in_type(&target, &cand.self_ty),
                    &subst,
                );
                let ta = apply_targs_subst(&ta, &subst);
                children.push(self.walk(&target, &tr, &ta)?);
            }
        }
        // A ranked supertrait (`LoopRanked<R>: BaseRanked<R>`) is demanded of the type too.
        for (sup, sup_targs) in ranked_supers(trait_, targs, table) {
            children.push(self.walk(ty, &sup, &sup_targs)?);
        }
        if !matched.is_empty() {
            self.entries[ix].kind = Kind::Node {
                children,
                impls: matched,
            };
            return Some(ix);
        }
        // No impl taken over implements it: a bridge, if one can be written without overlapping any.
        if crate::names::names_bare(ty, &self.heads) || !bridgeable_shape(ty) {
            return None;
        }
        if !trait_.items.iter().all(|it| {
            matches!(
                it,
                TraitItem::Fn(_) | TraitItem::Type(_) | TraitItem::Const(_)
            )
        }) {
            return None;
        }
        let own_vars: HashSet<Ident> = self.impl_params.clone();
        for cand in impls {
            if may_unify(
                &cand.self_ty,
                &crate::names::type_param_idents(&cand.generics),
                ty,
                &own_vars,
            ) {
                return None;
            }
        }
        self.entries[ix].kind = Kind::Bridge;
        self.bridges.push(self.bridge_req(ty, trait_ident));
        Some(ix)
    }

    fn bridge_req(&self, ty: &Type, trait_ident: &Ident) -> BridgeReq {
        let (lts, params) = mentioned_params(ty, &self.impl_.generics);
        let maybe_unsized = maybe_unsized_type_params(&self.impl_.generics);
        let mut renamer = crate::GenericRenamer {
            lifetime_renames: HashMap::new(),
            ident_renames: HashMap::new(),
        };
        let mut decl: Vec<GenericParam> = Vec::new();
        for (i, l) in lts.iter().enumerate() {
            let fresh = Lifetime::new(&format!("'__dcl_bl{}", i), l.span());
            decl.push(pq!(#fresh));
            renamer.lifetime_renames.insert(l.ident.to_string(), fresh);
        }
        let mut vars = HashSet::new();
        for (i, p) in params.iter().enumerate() {
            match p {
                GenericParam::Type(t) => {
                    let fresh = name!("DclBp{}", i);
                    if maybe_unsized.contains(&t.ident) {
                        decl.push(pq!(#fresh: ?::core::marker::Sized));
                    } else {
                        decl.push(pq!(#fresh));
                    }
                    vars.insert(fresh.clone());
                    renamer.ident_renames.insert(t.ident.to_string(), fresh);
                }
                GenericParam::Const(c) => {
                    let fresh = name!("DclBc{}", i);
                    let cty = &c.ty;
                    decl.push(pq!(const #fresh: #cty));
                    renamer.ident_renames.insert(c.ident.to_string(), fresh);
                }
                GenericParam::Lifetime(_) => {}
            }
        }
        let mut renamed = ty.clone();
        renamer.visit_type_mut(&mut renamed);
        BridgeReq {
            trait_ident: trait_ident.clone(),
            key: format!("{}|{}", trait_ident, quote!(#renamed)),
            ty: renamed,
            params: decl,
            cfgs: crate::extract_cfg_attrs(&self.impl_.attrs),
            vars,
        }
    }
}

/// What [`settle`] decided.
pub(super) struct Settled {
    /// The bridges to emit.
    pub bridges: Vec<BridgeReq>,
    /// The keys of the bridges available: those emitted, and those another one covers.
    keys: HashSet<String>,
    /// The impls a surviving plan goes through, `(trait, index among its impls)`: their floors rank
    /// their inline cyclic bounds ([`floor_generics`]).
    ranked_floors: HashSet<(String, usize)>,
}

impl Settled {
    /// Did `plan` survive: is every bridge it needs available?
    pub(super) fn admits(&self, plan: &Plan) -> bool {
        plan.bridges.iter().all(|b| self.keys.contains(&b.key))
    }

    /// Does the floor of the `index`-th impl of `trait_ident` rank its inline cyclic bounds?
    pub(super) fn ranked_floor(&self, trait_ident: &Ident, index: usize) -> bool {
        self.ranked_floors
            .contains(&(trait_ident.to_string(), index))
    }
}

/// The edges of `im` to plan: every bare routed-trait bound on a participant target other than `Self`
/// or a bare impl parameter, with whether it is a wrapper `peel` kept (`helper::contains_bare_member`
/// without a participant head). `(predicate index, bound index, target, trait, args, wrapper)`.
fn plannable_edges(
    im: &ItemImpl,
    table: &Table,
    heads: &HashSet<Ident>,
) -> Vec<(usize, usize, Type, Ident, Vec<GenericArgument>, bool)> {
    let params = crate::names::type_param_idents(&im.generics);
    let mut out = Vec::new();
    let Some(wc) = &im.generics.where_clause else {
        return out;
    };
    for (pi, pred) in wc.predicates.iter().enumerate() {
        let WherePredicate::Type(pt) = pred else {
            continue;
        };
        if is_bare_param(&pt.bounded_ty, &params) || is_self_type(&pt.bounded_ty) {
            continue;
        }
        let headed =
            crate::helper::type_head_ident(&pt.bounded_ty).is_some_and(|h| heads.contains(&h));
        let wrapper = !headed && crate::names::names_bare(&pt.bounded_ty, heads);
        if !headed && !wrapper {
            continue;
        }
        for (bi, b) in pt.bounds.iter().enumerate() {
            let TypeParamBound::Trait(tb) = b else {
                continue;
            };
            if tb.path.leading_colon.is_some() || tb.path.segments.len() != 1 {
                continue;
            }
            let seg = &tb.path.segments[0];
            if !table.contains_key(&seg.ident) {
                continue;
            }
            out.push((
                pi,
                bi,
                pt.bounded_ty.clone(),
                seg.ident.clone(),
                nonlifetime_path_args(&seg.arguments),
                wrapper,
            ));
        }
    }
    out
}

/// Plan every edge of every impl, decide which bridges can be emitted, and drop each wrapper bound
/// `peel` kept whose plan does not survive.
///
/// A bridge survives unless another bridge of the same trait may overlap it (two spellings that may
/// name one type, or a generic and a concrete one) or the impls needing one bridge disagree on their
/// `#[cfg]`s. A plan survives when it succeeded and every bridge it needs survives. A wrapper bound
/// whose plan does not survive is removed, which leaves the impl as `peel` would always have left
/// it; a bound headed by a participant is never removed (it is the impl's own edge), its plan only
/// decides whether the bridges it would need are emitted.
pub(super) fn settle(traits: &[ItemTrait], table: &mut Table) -> Settled {
    let heads = cycle_self_heads(table);
    let order: Vec<Ident> = {
        let mut seen = HashSet::new();
        traits
            .iter()
            .filter(|t| table.contains_key(&t.ident) && seen.insert(t.ident.clone()))
            .map(|t| t.ident.clone())
            .collect()
    };
    // (trait, impl index, pred, bound, wrapper, plan)
    let mut planned: Vec<(Ident, usize, usize, usize, bool, Option<Plan>)> = Vec::new();
    for tr in &order {
        let impls = &table[tr].2;
        for (ii, im) in impls.iter().enumerate() {
            for (pi, bi, target, trait_ident, targs, wrapper) in plannable_edges(im, table, &heads)
            {
                let plan = Planner::plan(table, im, &target, &trait_ident, &targs);
                planned.push((tr.clone(), ii, pi, bi, wrapper, plan));
            }
        }
    }
    // Bridge candidates, first appearance first.
    let mut cands: Vec<BridgeReq> = Vec::new();
    let mut bad: HashSet<String> = HashSet::new();
    for (_, _, _, _, _, plan) in &planned {
        let Some(plan) = plan else { continue };
        for b in &plan.bridges {
            match cands.iter().find(|c| c.key == b.key) {
                Some(c) => {
                    let cfg = |v: &[Attribute]| quote!(#(#v)*).to_string();
                    if cfg(&c.cfgs) != cfg(&b.cfgs) {
                        bad.insert(b.key.clone());
                    }
                }
                None => cands.push(b.clone()),
            }
        }
    }
    // Two bridges of one trait that may overlap: when one's type is an instance of the other's
    // (`Vec<u8>` of `Vec<P>`), the general one covers it; otherwise neither can be written.
    let mut covered: HashMap<String, String> = HashMap::new();
    let instance_of = |x: &BridgeReq, y: &BridgeReq| {
        let cfg = |v: &[Attribute]| quote!(#(#v)*).to_string();
        cfg(&x.cfgs) == cfg(&y.cfgs) && crate::unify::unify_type(&y.vars, &y.ty, &x.ty).is_some()
    };
    for (i, x) in cands.iter().enumerate() {
        for y in cands.iter().skip(i + 1) {
            if x.trait_ident != y.trait_ident || !may_unify(&x.ty, &x.vars, &y.ty, &y.vars) {
                continue;
            }
            if instance_of(x, y) {
                covered.insert(x.key.clone(), y.key.clone());
            } else if instance_of(y, x) {
                covered.insert(y.key.clone(), x.key.clone());
            } else {
                bad.insert(x.key.clone());
                bad.insert(y.key.clone());
            }
        }
    }
    // A bridge is available when it is fine itself and so is the one covering it (transitively).
    let available = |key: &String| {
        let mut k = key;
        for _ in 0..=cands.len() {
            if bad.contains(k) {
                return false;
            }
            match covered.get(k) {
                Some(next) => k = next,
                None => return true,
            }
        }
        false
    };
    let survives = |plan: &Option<Plan>| {
        plan.as_ref()
            .is_some_and(|p| p.bridges.iter().all(|b| available(&b.key)))
    };
    let mut keys: HashSet<String> = HashSet::new();
    let mut ranked_floors: HashSet<(String, usize)> = HashSet::new();
    let mut drop: Vec<(String, usize, usize, usize)> = Vec::new();
    for (tr, ii, pi, bi, wrapper, plan) in &planned {
        if survives(plan) {
            let plan = plan.as_ref().unwrap();
            for b in &plan.bridges {
                // A covered bridge stands for the one covering it, which is emitted in its place.
                let mut k = &b.key;
                keys.insert(k.clone());
                while let Some(next) = covered.get(k) {
                    k = next;
                    keys.insert(k.clone());
                }
            }
            for e in &plan.entries {
                if let Kind::Node { impls, .. } = &e.kind {
                    ranked_floors.extend(impls.iter().map(|ix| (e.trait_ident.to_string(), *ix)));
                }
            }
        } else if *wrapper {
            drop.push((tr.to_string(), *ii, *pi, *bi));
        }
    }
    // Drop the failed wrapper bounds, last first so that the indices stay valid.
    drop.sort();
    for (tr, ii, pi, bi) in drop.into_iter().rev() {
        let key = table.keys().find(|k| k.to_string() == tr).unwrap().clone();
        let im = &mut table.get_mut(&key).unwrap().2[ii];
        let wc = im.generics.where_clause.as_mut().unwrap();
        let mut preds: Vec<WherePredicate> =
            std::mem::take(&mut wc.predicates).into_iter().collect();
        if let WherePredicate::Type(pt) = &mut preds[pi] {
            let mut bounds: Vec<TypeParamBound> =
                std::mem::take(&mut pt.bounds).into_iter().collect();
            bounds.remove(bi);
            pt.bounds = bounds.into_iter().collect();
            if pt.bounds.is_empty() {
                preds.remove(pi);
            }
        }
        wc.predicates = preds.into_iter().collect();
    }
    let bridges = cands
        .into_iter()
        .filter(|b| keys.contains(&b.key) && !covered.contains_key(&b.key))
        .collect();
    Settled {
        bridges,
        keys,
        ranked_floors,
    }
}

/// The bridge `b` of `trait_`: a ranked impl at every rank that delegates every item to the real
/// trait. Emitted inside `ranked_traits`, where the ranked trait is a sibling and the real one is
/// `original_trait_path`.
pub(super) fn emit_bridge(trait_: &ItemTrait, rank_loc: usize, b: &BridgeReq) -> TokenStream {
    let rank = name!("Rank");
    let ranked = name!("{}Ranked", &trait_.ident);
    let ranked_args = trait_.generics.ty_generics().insert(rank_loc, pq!(#rank));
    let real = original_trait_path(trait_);
    let ty = &b.ty;
    let trait_lts: Vec<&GenericParam> = trait_
        .generics
        .params
        .iter()
        .filter(|p| matches!(p, GenericParam::Lifetime(_)))
        .collect();
    let trait_rest: Vec<&GenericParam> = trait_
        .generics
        .params
        .iter()
        .filter(|p| !matches!(p, GenericParam::Lifetime(_)))
        .collect();
    let own_lts: Vec<&GenericParam> = b
        .params
        .iter()
        .filter(|p| matches!(p, GenericParam::Lifetime(_)))
        .collect();
    let own_rest: Vec<&GenericParam> = b
        .params
        .iter()
        .filter(|p| !matches!(p, GenericParam::Lifetime(_)))
        .collect();
    // A where-clause only when it is not global: on a type and trait with no parameter at all, a
    // false global bound is the `trivial_bounds` error, and the body states the obligation anyway.
    let generic = !b.params.is_empty() || !trait_.generics.params.is_empty();
    let trait_where: Vec<&WherePredicate> = trait_
        .generics
        .where_clause
        .iter()
        .flat_map(|w| w.predicates.iter())
        .collect();
    let mut items = TokenStream::new();
    for item in &trait_.items {
        match item {
            TraitItem::Fn(tf) => {
                let mut sig = tf.sig.clone();
                replace_self_and_desugar_impl_trait(&mut sig, &pq!(Self));
                for (ix, input) in sig.inputs.iter_mut().enumerate() {
                    input.reduce_pat(ix);
                }
                let margs = type_const_idents(&tf.sig.generics);
                let turbofish = !margs.is_empty() && !sig_has_impl_trait_input(&tf.sig);
                let call = quote! {
                    <Self as #real>::#{&sig.ident}
                    #(if turbofish) { ::<#(#margs),*> }
                    (#(for input in &sig.inputs), { #{input.variable()} })
                };
                let cfgs = crate::extract_cfg_attrs(&tf.attrs);
                items.extend(quote! {
                    #(for a in &cfgs) { #a }
                    #[inline]
                    #sig {
                        #(if sig.unsafety.is_some()) { unsafe { #call } }
                        #(else) { #call }
                    }
                });
            }
            TraitItem::Type(tt) => {
                let ident = &tt.ident;
                let generics = &tt.generics;
                let targs = generics.ty_generics();
                let cfgs = crate::extract_cfg_attrs(&tt.attrs);
                items.extend(quote! {
                    #(for a in &cfgs) { #a }
                    type #ident #generics = <Self as #real>::#ident #targs #{&generics.where_clause};
                });
            }
            TraitItem::Const(tc) => {
                let ident = &tc.ident;
                let cty = &tc.ty;
                let cfgs = crate::extract_cfg_attrs(&tc.attrs);
                items.extend(quote! {
                    #(for a in &cfgs) { #a }
                    const #ident: #cty = <Self as #real>::#ident;
                });
            }
            _ => {}
        }
    }
    quote! {
        #(for a in &b.cfgs) { #a }
        #[allow(unused_mut, unused_variables, clippy::all)]
        impl<
            #(for p in &trait_lts) { #{param_decl(p, DeclBounds::Keep)}, }
            #(for p in &own_lts) { #p, }
            #rank,
            #(for p in &trait_rest) { #{param_decl(p, DeclBounds::Keep)}, }
            #(for p in &own_rest) { #p, }
        > #ranked #ranked_args for #ty
        #(if generic) {
            where
                #ty: #real,
                #(for w in &trait_where) { #w, }
        }
        {
            #items
        }
    }
}

/// The plan entries reached from entry `ix`, itself excluded, in first-reached order.
pub(super) fn reached(plan: &Plan) -> Vec<usize> {
    let mut seen = vec![false; plan.entries.len()];
    let mut out = Vec::new();
    let mut stack = vec![0usize];
    seen[0] = true;
    while let Some(ix) = stack.pop() {
        if let Kind::Node { children, .. } = &plan.entries[ix].kind {
            for &c in children.iter().rev() {
                if !seen[c] {
                    seen[c] = true;
                    out.push(c);
                    stack.push(c);
                }
            }
        }
    }
    out
}

/// May the frame of `impl_` register the re-entry fn of plan entry `ix`? Its registration names
/// `ty: Tr` on the REAL trait inside the register-once fn, so it must be provable from `impl_`'s own
/// side bounds: the entry's own chain (`reachable_side_bounds_ok`), and — since that walk does not
/// descend a bare parameter — every entry its ranked copies demand: a bridged type with no
/// parameter (its impl is the user's), or one whose premise `impl_` states as written; a node, the
/// same way; never a bare parameter of `impl_` (its real bound is stripped there).
pub(super) fn registration_ok(impl_: &ItemImpl, plan: &Plan, ix: usize, table: &Table) -> bool {
    let own_side: HashSet<String> = side_predicate_strings(&impl_.generics, table, None)
        .into_iter()
        .collect();
    let params = crate::names::type_param_idents(&impl_.generics);
    let mut memo: HashMap<usize, bool> = HashMap::new();
    fn go(
        impl_: &ItemImpl,
        plan: &Plan,
        ix: usize,
        table: &Table,
        own_side: &HashSet<String>,
        params: &HashSet<Ident>,
        memo: &mut HashMap<usize, bool>,
    ) -> bool {
        if let Some(&v) = memo.get(&ix) {
            return v;
        }
        memo.insert(ix, true); // a cycle back to an entry being checked holds by that entry
        let e = &plan.entries[ix];
        let ok = match &e.kind {
            Kind::Param => false,
            Kind::Bridge => {
                let ty = &e.ty;
                !mentions(ty, params) || {
                    let tr = &e.trait_ident;
                    let targs = &e.targs;
                    let bound: TraitBound = pq!(#tr #(if !targs.is_empty()) { <#(#targs),*> });
                    own_side.contains(&format_side_bound(ty, &bound, None))
                }
            }
            Kind::Node { children, .. } => {
                reachable_side_bounds_ok(impl_, &e.ty, &e.trait_ident, &e.targs, table)
                    && children
                        .iter()
                        .all(|&c| go(impl_, plan, c, table, own_side, params, memo))
            }
        };
        memo.insert(ix, ok);
        ok
    }
    go(impl_, plan, ix, table, &own_side, &params, &mut memo)
}

/// Does an impl taken over (`adopted`, self types in their bare spelling) implement the outer shape
/// of `ty` for `bound`? `peel`'s test for keeping a wrapper whole: the bound must name a routed
/// trait bare, with plain arguments only (an associated-type binding is no wrapper edge `validate`
/// would let through), and some impl of that trait must match `ty` — self type and trait arguments
/// — as `reachable_side_bounds_ok` matches it.
pub(crate) fn shape_matches(adopted: &[ItemImpl], ty: &Type, bound: &TypeParamBound) -> bool {
    let TypeParamBound::Trait(tb) = bound else {
        return false;
    };
    if tb.lifetimes.is_some() || tb.path.leading_colon.is_some() || tb.path.segments.len() != 1 {
        return false;
    }
    let seg = &tb.path.segments[0];
    let plain = match &seg.arguments {
        PathArguments::None => true,
        PathArguments::AngleBracketed(ab) => ab.args.iter().all(|a| {
            matches!(
                a,
                GenericArgument::Type(_) | GenericArgument::Lifetime(_) | GenericArgument::Const(_)
            )
        }),
        PathArguments::Parenthesized(_) => false,
    };
    if !plain {
        return false;
    }
    let targs = nonlifetime_path_args(&seg.arguments);
    adopted.iter().any(|im| {
        let Some((_, path, _)) = im.trait_.as_ref() else {
            return false;
        };
        let mut path = path.clone();
        crate::names::strip_leading_self(&mut path);
        if path.segments.len() != 1 || path.segments[0].ident != seg.ident {
            return false;
        }
        let vars = crate::names::type_param_idents(&im.generics);
        let im_targs = nonlifetime_path_args(&path.segments[0].arguments);
        crate::unify::unify_obligation(&vars, &im.self_ty, &im_targs, ty, &targs).is_some()
    })
}
