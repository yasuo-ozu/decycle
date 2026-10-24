//! Share the premises of a cycle's impls between them — both engines, through [`share_premises`].
//!
//! An impl's generated code reaches each impl its cyclic bounds name through the public trait, and
//! that impl demands its own premises. Plain Rust proves them through the cyclic bound itself; the
//! engines strip (structural) or rank (ranked) that bound, so the impl needs them in its own
//! environment. Two passes give them, each scoped by the engine (which impls are a cycle's, and
//! which bounds are its edges, `is_edge`, are the engine's to say):
//!
//! - **by name** ([`share_side_predicates_scoped`]): the impls of one trait take the union of each
//!   other's premises, computed from their *real* predicates rather than re-derived from field
//!   types, so an obligation a caller introduced by hand (or through a generated helper type) is
//!   covered too. A cycle's own edges are never shared: the ranked engine ranks them, and copying
//!   one onto a sibling would rank-lower it there as well; the structural engine strips them. Which
//!   impls an impl may inherit from (`sources`) is the engine's too: an impl of the same trait that
//!   shares no obligation path — a standalone acyclic impl, or a member of a *different*, disjoint
//!   cycle — receives nothing, so it does not silently acquire premises (`C<T>` acquiring
//!   `T: Clone`) that reject code valid without the macro.
//! - **along the bounds** ([`mapped_premises`]): by name never reaches an impl of another trait in
//!   a cycle that alternates between traits (`Expr: Eval` needing `Term: Size`), and it cannot map
//!   a parameter that is named differently on the two sides. This pass follows the bounds instead:
//!
//! - an impl `J` is reached by the bound `X: Tr<..>` of an impl `I` when `X` names `J`'s self type
//!   and `Tr` its trait, and `J`'s header **matches** the bound: `J`'s self-type and trait arguments,
//!   with `J`'s generic parameters as the variables, are matched against the bound's. The match is
//!   the mapping: `impl<U> Size<U> for Term` against `Term: Size<T>` maps `U` to `T`, and
//!   `impl<R> Size<u8> for Term<R>` against `Term<S>: Size<T>` does not match at all (the bound is
//!   then discharged by no impl this pass can see, and nothing flows through it);
//! - `J`'s premises, with `J`'s parameters replaced by what the match maps them to, become premises
//!   of `I`, and so on transitively: what `J` received flows on to the impls reaching `J`.
//!
//! What `I` receives is what its cyclic bound already required of every instantiation, only spelled
//! in `I`'s terms: proving `X: Tr<..>` selects `J` (the header matched; by coherence no other impl
//! applies) at the mapped instantiation, which demands those premises. So the added bounds change
//! nothing an impl effectively requires. For a wrapped bound (`Box<Term>: Size<T>`) that rests on
//! the contract both engines read such a bound by, that the wrapper implements the trait when its
//! argument does (the structural engine asserts it). A lifetime the bound binds
//! (`for<'a> Term<'a>: Size<T>`) is bound on the premise as well (`for<'a> T: 'a`).
//!
//! A premise that cannot be expressed in `I`'s terms is skipped, never copied as written: one naming
//! a parameter of `J` the match leaves open (a lifetime of `J`'s that no argument of the bound fixes,
//! `impl<'u, T> .. where T: 'u`), one naming a concrete item that `I` shadows with a parameter of the
//! same name, a projection `T::Out` whose `T` maps to a type that is no plain name, a premise behind a
//! macro, and a `Self` that is not the whole type (`Self::Out`). Nothing is reported: the premise is
//! then only missing as it was before, and if `I` needed it rustc reports the unproven bound at the
//! user's cyclic bound. A premise is one bound: `T: Clone + 'u` is read as `T: Clone` (shared) and
//! `T: 'u` (not), and inline parameter bounds (`impl<T: Clone>`) count as premises too. The ranked
//! engine takes fewer of them ([`Respell::TypeParams`]).
//!
//! A premise travels along at most `2n + 2` bounds (`n` the number of impls): every simple path,
//! and the extra laps a cycle needs whose mapping permutes its parameters. The limit only matters to
//! a *growing* cycle (`Term: Size<Vec<T>>` back to `Expr: Eval<Vec<T>>`), whose premises would
//! otherwise grow forever; no engine can monomorphise such a cycle anyway.

use std::collections::{HashMap, HashSet, VecDeque};
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;
use syn::{
    BoundLifetimes, GenericArgument, GenericParam, ItemImpl, Lifetime, LifetimeParam, Path,
    PathArguments, PredicateLifetime, PredicateType, TraitBound, TraitBoundModifier, Type,
    TypeParamBound, TypePath, WherePredicate,
};
use template_quote::quote;

use crate::names::{bare_ident, local_type_head_ident, strip_parens};
use crate::unify::{parenthesize, spelling, Apply, Matcher, Names, Params, Reading, Subst};

/// One cyclic bound of an impl, as this pass follows it: the bound `trait_path` on `target`, a cycle
/// member's type. A wrapped bound (`Box<Term<S>>: Size<T>`) is followed to the member inside it
/// (`Term<S>`), as the engines read it: a wrapper implements the trait when its arguments do.
pub(crate) struct CycleEdge {
    /// The member's type as the bound spells it, `Self` resolved to the impl's self type.
    pub target: Type,
    /// The bound's trait, with its arguments (`Size<T>`).
    pub trait_path: Path,
    /// The lifetimes the predicate (`for<'x> Term<'x>: ..`) or the bound (`Term: for<'x> ..`) binds.
    pub binder: Vec<Lifetime>,
    /// The impls of the group (indices) that may discharge the bound: those of its trait for the
    /// member's type. Each one whose header matches the bound is reached.
    pub candidates: Vec<usize>,
}

impl CycleEdge {
    /// The edge the bound `tb` of the predicate `pt` makes to the member `target` (`Self`
    /// resolved), which the impls of `impls` of the bound's trait for the member's head may
    /// discharge — both read by their bare names ([`crate::names`]), as both engines take an impl
    /// over.
    pub(crate) fn new(
        target: Type,
        pt: &PredicateType,
        tb: &TraitBound,
        impls: &[&ItemImpl],
    ) -> Self {
        let trait_ident = bare_ident(&tb.path);
        let head = local_type_head_ident(&target);
        let candidates = impls
            .iter()
            .enumerate()
            .filter(|(_, j)| {
                trait_ident.is_some()
                    && j.trait_.as_ref().and_then(|(_, path, _)| bare_ident(path)) == trait_ident
                    && head.is_some()
                    && local_type_head_ident(&j.self_ty) == head
            })
            .map(|(k, _)| k)
            .collect();
        let binder = binder_lifetimes(pt.lifetimes.as_ref())
            .chain(binder_lifetimes(tb.lifetimes.as_ref()))
            .collect();
        CycleEdge {
            target,
            trait_path: tb.path.clone(),
            binder,
            candidates,
        }
    }
}

/// The lifetimes a `for<..>` binds.
fn binder_lifetimes(binder: Option<&BoundLifetimes>) -> impl Iterator<Item = Lifetime> + '_ {
    binder
        .into_iter()
        .flat_map(|b| &b.lifetimes)
        .filter_map(|p| match p {
            GenericParam::Lifetime(l) => Some(l.lifetime.clone()),
            _ => None,
        })
}

/// How the impl receiving a premise may spell it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Respell {
    /// In whatever way says the same thing in its terms: lifetimes renamed, a binder renamed apart
    /// or added, `Self` spelled as the reached impl's self type (the structural engine).
    Any,
    /// Only as the ranked engine's re-entry registration check (`reachable_side_bounds_ok`) spells
    /// it. That check maps a reached impl's premises back into the terms of the impl reaching it
    /// through the same matcher, read as [`Reading::Registration`]: it substitutes type parameters
    /// only (in the bounded type and in the bound's `<..>` type arguments) and compares the result
    /// by spelling, where this pass maps lifetimes and const parameters too and respells what it
    /// substitutes ([`Reading::Premises`]). A premise it would spell differently from this pass
    /// would not be recognised there, and the registration of an impl that registered before would
    /// now be skipped. So a premise with a trait bound is only shared when the two spellings agree
    /// under any substitution: no lifetime but `'static` and no `for<..>`, no const parameter, no
    /// projection on a parameter (`T::Out`), no parameter in `Fn(..)` sugar, an associated-item
    /// constraint or a const argument of the bound; and a premise naming `Self` is not shared at
    /// all (the check reads `Self` as written: `Self: Debug` would not recognise `Term<R>: Debug`).
    /// A premise with lifetime bounds only (`T: 'a`, `'a: 'b`) is never compared, and is shared.
    TypeParams,
}

/// Share the premises of `impls`, in place, by both passes (see the module docs); whether any impl
/// changed.
///
/// `edges[i]` are the cyclic bounds of `impls[i]`, and `is_edge` tells a cyclic bound from a
/// premise, as the engine reads them. `by_name` are the groups of the by-name pass: the indices
/// (into `impls`, ascending) of the impls of one trait, with each one's `sources`, indexed the same
/// way ([`share_side_predicates_scoped`]). The premises along the bounds are computed on the impls
/// as written, before the by-name pass adds to them, and added after it.
pub(crate) fn share_premises(
    impls: &mut [&mut ItemImpl],
    edges: &[Vec<CycleEdge>],
    is_edge: &dyn Fn(&ItemImpl, &WherePredicate) -> bool,
    respell: Respell,
    by_name: &[(Vec<usize>, Vec<Vec<usize>>)],
) -> bool {
    let along_bounds = {
        let written: Vec<&ItemImpl> = impls.iter().map(|im| &**im).collect();
        mapped_premises(&written, edges, is_edge, respell)
    };
    let mut changed = false;
    for (members, sources) in by_name {
        let mut group: Vec<&mut ItemImpl> = impls
            .iter_mut()
            .enumerate()
            .filter(|(k, _)| members.contains(k))
            .map(|(_, im)| &mut **im)
            .collect();
        changed |= share_side_predicates_scoped(&mut group, is_edge, sources);
    }
    for (im, add) in impls.iter_mut().zip(along_bounds) {
        changed |= add_premises(im, add);
    }
    changed
}

/// The premises each impl of `impls` receives along `edges` (`edges[i]` the cyclic bounds of
/// `impls[i]`), in its own terms, without the ones it states already. `is_edge` tells a cyclic bound
/// from a premise, as the engine reads them; neither a cyclic bound nor a premise that would read as
/// one in the receiving impl is shared.
fn mapped_premises(
    impls: &[&ItemImpl],
    edges: &[Vec<CycleEdge>],
    is_edge: &dyn Fn(&ItemImpl, &WherePredicate) -> bool,
    respell: Respell,
) -> Vec<Vec<WherePredicate>> {
    debug_assert_eq!(impls.len(), edges.len());
    let params: Vec<Params> = impls.iter().map(|im| Params::of(im)).collect();
    // `reached_by[j]`: the impls whose bounds reach `impls[j]`, with the mapping and the binder.
    let mut reached_by: Vec<Vec<(usize, Subst, &[Lifetime])>> = vec![Vec::new(); impls.len()];
    for (i, im_edges) in edges.iter().enumerate() {
        for edge in im_edges {
            for &j in &edge.candidates {
                if let Some(subst) = match_header(impls[j], &params[j], edge, &params[i]) {
                    reached_by[j].push((i, subst, &edge.binder));
                }
            }
        }
    }
    let mut need: Vec<Premises> = impls
        .iter()
        .map(|im| {
            let mut own = Premises::default();
            for p in own_premises(im, is_edge, respell == Respell::Any) {
                own.insert(p);
            }
            own
        })
        .collect();
    let own_keys: Vec<HashSet<String>> = need.iter().map(|n| n.keys.clone()).collect();
    let max_hops = 2 * impls.len() + 2;
    let mut queue: VecDeque<(usize, WherePredicate, usize)> = VecDeque::new();
    for (j, own) in need.iter().enumerate() {
        for p in &own.list {
            queue.push_back((j, p.clone(), 0));
        }
    }
    while let Some((j, p, hops)) = queue.pop_front() {
        if hops >= max_hops {
            continue;
        }
        for (i, subst, binder) in &reached_by[j] {
            let Some(q) = translate(&p, &params[j], subst, binder, &params[*i]) else {
                continue;
            };
            if is_edge(impls[*i], &q)
                || (respell == Respell::TypeParams && !spelled_alike_by_ranked(&q, &params[*i]))
            {
                continue;
            }
            if need[*i].insert(q.clone()) {
                queue.push_back((*i, q, hops + 1));
            }
        }
    }
    need.into_iter()
        .zip(own_keys)
        .map(|(n, own)| {
            n.list
                .into_iter()
                .filter(|p| !own.contains(&key(p)))
                .collect()
        })
        .collect()
}

/// Add `premises` to `im`, but those it states already (as a `where`-predicate, one bound of one, or
/// an inline parameter bound); whether anything was added. A premise on a parameter that has inline
/// bounds joins them (`impl<T: Clone>` given `T: Default` becomes `impl<T: Clone + Default>`): a
/// parameter bounded in both places trips `clippy::multiple_bound_locations` in the user's crate,
/// on the user's own impl header. Anything else goes to the `where`-clause.
fn add_premises(im: &mut ItemImpl, premises: Vec<WherePredicate>) -> bool {
    let mut have: HashSet<String> = inline_bound_predicates(im).iter().map(key).collect();
    for pred in im
        .generics
        .where_clause
        .iter()
        .flat_map(|wc| &wc.predicates)
    {
        have.insert(key(pred));
        for single in split(pred) {
            have.insert(key(&single));
        }
    }
    let add: Vec<WherePredicate> = premises
        .into_iter()
        .filter(|p| have.insert(key(p)))
        .collect();
    if add.is_empty() {
        return false;
    }
    for p in add {
        if !add_inline(&mut im.generics, &p) {
            im.generics
                .where_clause
                .get_or_insert_with(|| syn::parse_quote!(where))
                .predicates
                .push(p);
        }
    }
    true
}

/// Add `p` to the inline bounds of the parameter it bounds, if that parameter has inline bounds.
fn add_inline(generics: &mut syn::Generics, p: &WherePredicate) -> bool {
    match p {
        WherePredicate::Type(pt) if pt.lifetimes.is_none() => {
            let Type::Path(TypePath { qself: None, path }) = &pt.bounded_ty else {
                return false;
            };
            let Some(ident) = path.get_ident() else {
                return false;
            };
            generics.params.iter_mut().any(|param| match param {
                GenericParam::Type(tp) if tp.ident == *ident && !tp.bounds.is_empty() => {
                    tp.bounds.extend(pt.bounds.iter().cloned());
                    true
                }
                _ => false,
            })
        }
        WherePredicate::Lifetime(pl) => generics.params.iter_mut().any(|param| match param {
            GenericParam::Lifetime(lp) if lp.lifetime == pl.lifetime && !lp.bounds.is_empty() => {
                lp.bounds.extend(pl.bounds.iter().cloned());
                true
            }
            _ => false,
        }),
        _ => false,
    }
}

/// A set of premises in insertion order, keyed by spelling.
#[derive(Default)]
struct Premises {
    list: Vec<WherePredicate>,
    keys: HashSet<String>,
}

impl Premises {
    fn insert(&mut self, p: WherePredicate) -> bool {
        if self.keys.insert(key(&p)) {
            self.list.push(p);
            true
        } else {
            false
        }
    }
}

/// A premise's spelling, up to the names of the lifetimes its `for<..>` binders bind:
/// `for<'x> &'x T: Debug` and `for<'y> &'y T: Debug` are one premise.
fn key(p: &WherePredicate) -> String {
    #[derive(Default)]
    struct Order(Vec<String>);
    impl Visit<'_> for Order {
        fn visit_bound_lifetimes(&mut self, b: &BoundLifetimes) {
            for p in &b.lifetimes {
                if let GenericParam::Lifetime(l) = p {
                    let name = l.lifetime.to_string();
                    if !self.0.contains(&name) {
                        self.0.push(name);
                    }
                }
            }
            syn::visit::visit_bound_lifetimes(self, b);
        }
    }
    let mut order = Order::default();
    order.visit_where_predicate(p);
    if order.0.is_empty() {
        return spelling(p);
    }
    let renames: HashMap<String, Lifetime> = order
        .0
        .iter()
        .enumerate()
        .map(|(k, name)| {
            let canonical = format!("'__decycle_b{k}");
            (
                name.clone(),
                Lifetime::new(&canonical, proc_macro2::Span::call_site()),
            )
        })
        .collect();
    let mut p = p.clone();
    RenameLifetimes(&renames).visit_where_predicate_mut(&mut p);
    spelling(&p)
}

// ----- by name ------------------------------------------------------------------------------

/// The by-name pass over one trait's impls — see the module docs; whether any impl changed.
///
/// A cycle member's body reaches its siblings through the *public* trait (`Vec<Expr<S>>: Unparse<A>`
/// ⇒ `Expr<S>: Unparse<A>`), which decycle discharges through `Expr`'s delegating impl — and that
/// impl carries `Expr`'s **own** leaf bounds. A sibling (or a `#[group]` substruct) whose leaf set is
/// smaller therefore cannot prove it from its own `where`-clause. Sharing the union closes that gap;
/// it is the same idea as the `#[predicate_unparse(<leaf union>)]` injection the direct
/// `Unparse`/`Spanned` path uses, computed from the real predicates rather than re-derived from field
/// types (so a `#[group]`'s `Fill` obligation is covered too).
///
/// `sources[i]` lists the indices (into `impls`) of the impls whose premises `impls[i]` may inherit —
/// the impls it can actually reach, as computed by the caller from the obligation edges.
/// `is_edge(im, pred)` says whether a predicate of `im` is an edge of the cycle rather than a premise
/// — never shared (see the ranked engine's `is_ranked_edge` and the structural engine's
/// `is_cycle_edge`).
///
/// A predicate is only injected into an impl that declares every generic parameter it mentions, so a
/// cycle whose members carry *different* parameters stays well-formed.
fn share_side_predicates_scoped(
    impls: &mut [&mut ItemImpl],
    is_edge: &dyn Fn(&ItemImpl, &WherePredicate) -> bool,
    sources: &[Vec<usize>],
) -> bool {
    debug_assert_eq!(impls.len(), sources.len());
    let mut changed = false;
    // Per-impl data up front — the injection loop below holds the mutable borrow.
    let param_sets: Vec<HashSet<String>> = impls.iter().map(|im| impl_param_idents(im)).collect();
    let shareable: Vec<Vec<WherePredicate>> = impls
        .iter()
        .map(|im| {
            im.generics
                .where_clause
                .iter()
                .flat_map(|wc| wc.predicates.iter())
                // Skip the cycle's edges: those are the engine's to rank or strip.
                .filter(|p| !is_edge(im, p))
                // `Self` is impl-relative: `Self: Debug` on `impl Tr for A` says *A* is `Debug`, so
                // copying it onto `impl Tr for B` silently asserts something about a different type
                // (and fails if B is not `Debug`). Only impl-independent premises can be shared.
                .filter(|p| !Names::of_predicate(p).idents.contains("Self"))
                .cloned()
                .collect()
        })
        .collect();
    for (i, im) in impls.iter_mut().enumerate() {
        // The union this impl may inherit, from its sources only.
        let mut union: Vec<WherePredicate> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for j in sources[i].iter() {
            for p in &shareable[*j] {
                if seen.insert(quote!(#p).to_string()) {
                    union.push(p.clone());
                }
            }
        }
        if union.is_empty() {
            continue;
        }
        // Idents that are a generic parameter of this impl or of one of its sources — used below to
        // tell a parameter apart from a concrete type name. Scoped to the receiver's own slice of the
        // group on purpose: it was previously a `thread_local!` that accumulated and was never
        // cleared, so parameters leaked between separate `#[decycle]` expansions compiled on the same
        // thread, making unrelated idents look param-like and silently suppressing predicate sharing.
        let group_params: HashSet<String> = sources[i]
            .iter()
            .flat_map(|j| param_sets[*j].iter())
            .chain(param_sets[i].iter())
            .cloned()
            .collect();
        let params = &param_sets[i];
        let lifetimes: HashSet<String> = im
            .generics
            .params
            .iter()
            .filter_map(|p| match p {
                syn::GenericParam::Lifetime(l) => Some(l.lifetime.ident.to_string()),
                _ => None,
            })
            .collect();
        let have: HashSet<String> = im
            .generics
            .where_clause
            .iter()
            .flat_map(|wc| wc.predicates.iter())
            .map(|p| quote!(#p).to_string())
            .collect();
        let mut add: Vec<WherePredicate> = Vec::new();
        for p in &union {
            if have.contains(&quote!(#p).to_string()) {
                continue;
            }
            // Every *declared* parameter the predicate names must exist here too. An ident that is
            // not a parameter anywhere (a concrete type, a trait) is not a constraint.
            let idents = pred_idents(p);
            let missing = idents
                .iter()
                .any(|i| !params.contains(i) && group_params.contains(i));
            if missing {
                continue;
            }
            // A shared HRTB predicate (`for<'a> <G as EmptyGroup>::Fill<Sub<'a, ..>>: …`) can carry a
            // binder lifetime that the receiving impl already declares as a parameter — a `#[group]`
            // substruct's impl declares exactly the lifetime its own predicate binds. Rename the
            // binder rather than shadowing it (E0496); an HRTB binder name is not observable.
            add.push(freshen_binder_lifetimes(p.clone(), &lifetimes));
        }
        if add.is_empty() {
            continue;
        }
        let wc = im
            .generics
            .where_clause
            .get_or_insert_with(|| syn::parse_quote!(where));
        for p in add {
            wc.predicates.push(p);
        }
        changed = true;
    }
    changed
}

/// The generic-parameter idents an impl declares — used to decide whether a sibling's predicate can be
/// injected into it (see [`share_side_predicates_scoped`]).
pub(crate) fn impl_param_idents(item_impl: &ItemImpl) -> HashSet<String> {
    item_impl
        .generics
        .params
        .iter()
        .map(|p| match p {
            syn::GenericParam::Type(t) => t.ident.to_string(),
            syn::GenericParam::Const(c) => c.ident.to_string(),
            // Lifetimes count too. Leaving them out let a predicate naming a SIBLING's lifetime
            // (`where &'a str: Clone`) past the "declares everything it mentions" guard and into
            // an impl with no `'a` of its own — E0261 on code that is valid without the macro.
            // `freshen_binder_lifetimes` does not help: it renames `for<>` binders, not free
            // lifetimes.
            syn::GenericParam::Lifetime(l) => l.lifetime.ident.to_string(),
        })
        .collect()
}

/// Every single-segment ident appearing anywhere in a predicate.
fn pred_idents(pred: &WherePredicate) -> HashSet<String> {
    struct V(HashSet<String>);
    impl syn::visit::Visit<'_> for V {
        fn visit_ident(&mut self, i: &proc_macro2::Ident) {
            self.0.insert(i.to_string());
        }
        // `'static` and `'_` are always in scope, so they must not count as something the
        // receiving impl has to declare — otherwise, now that `impl_param_idents` reports
        // lifetimes, an ordinary `T: 'static` co-bound would never be shared. Deliberately does
        // not recurse: the default impl would feed the bare ident to `visit_ident`.
        fn visit_lifetime(&mut self, lt: &syn::Lifetime) {
            if lt.ident != "static" && lt.ident != "_" {
                self.0.insert(lt.ident.to_string());
            }
        }
    }
    let mut v = V(HashSet::new());
    syn::visit::Visit::visit_where_predicate(&mut v, pred);
    v.0
}

/// Rename a predicate's `for<'a, …>` binder lifetimes that collide with `taken`.
fn freshen_binder_lifetimes(mut pred: WherePredicate, taken: &HashSet<String>) -> WherePredicate {
    let WherePredicate::Type(pt) = &mut pred else {
        return pred;
    };
    let mut rename: HashMap<String, syn::Lifetime> = HashMap::new();
    if let Some(bl) = &mut pt.lifetimes {
        for p in bl.lifetimes.iter_mut() {
            if let syn::GenericParam::Lifetime(l) = p {
                let old = l.lifetime.ident.to_string();
                if taken.contains(&old) {
                    let fresh = syn::Lifetime::new(
                        &format!("'{}__decycle_hr", old),
                        l.lifetime.ident.span(),
                    );
                    rename.insert(l.lifetime.to_string(), fresh.clone());
                    l.lifetime = fresh;
                }
            }
        }
    }
    // The binder itself is renamed above; only the predicate's type and bounds are rewritten.
    if !rename.is_empty() {
        let mut r = RenameLifetimes(&rename);
        if let WherePredicate::Type(pt) = &mut pred {
            r.visit_type_mut(&mut pt.bounded_ty);
            for b in pt.bounds.iter_mut() {
                r.visit_type_param_bound_mut(b);
            }
        }
    }
    pred
}

// ----- premises -------------------------------------------------------------------------------

/// `im`'s inline parameter bounds as the `where`-predicates they are equivalent to, one bound each
/// (`impl<T: Clone + Debug>` → `T: Clone`, `T: Debug`).
fn inline_bound_predicates(im: &ItemImpl) -> Vec<WherePredicate> {
    let mut out = Vec::new();
    for p in &im.generics.params {
        match p {
            GenericParam::Type(t) => {
                let ident = &t.ident;
                for b in &t.bounds {
                    out.push(syn::parse_quote!(#ident: #b));
                }
            }
            GenericParam::Lifetime(l) => {
                let lt = &l.lifetime;
                for b in &l.bounds {
                    out.push(syn::parse_quote!(#lt: #b));
                }
            }
            GenericParam::Const(_) => {}
        }
    }
    out
}

/// A predicate as one predicate per bound: `T: Clone + 'u` → `T: Clone`, `T: 'u`.
fn split(pred: &WherePredicate) -> Vec<WherePredicate> {
    match pred {
        WherePredicate::Type(pt) => pt
            .bounds
            .iter()
            .map(|b| {
                let mut one = pt.clone();
                one.bounds = Punctuated::new();
                one.bounds.push(b.clone());
                WherePredicate::Type(one)
            })
            .collect(),
        WherePredicate::Lifetime(pl) => pl
            .bounds
            .iter()
            .map(|b| {
                let mut one: PredicateLifetime = pl.clone();
                one.bounds = Punctuated::new();
                one.bounds.push(b.clone());
                WherePredicate::Lifetime(one)
            })
            .collect(),
        other => vec![other.clone()],
    }
}

/// A relaxed bound (`T: ?Sized`) withdraws a requirement rather than stating one: no premise.
fn is_relaxed(pred: &WherePredicate) -> bool {
    matches!(pred, WherePredicate::Type(PredicateType { bounds, .. })
        if bounds.iter().all(|b| matches!(b, TypeParamBound::Trait(tb)
            if matches!(tb.modifier, TraitBoundModifier::Maybe(_)))))
}

/// `im`'s premises, one bound each, in its own terms with `Self` spelled as its self type: its inline
/// parameter bounds and `where`-predicates, but the cyclic bounds (`is_edge`) and relaxed bounds. A
/// premise naming `Self` other than as a whole type (`Self::Out`), or at all unless `share_self`, is
/// left out.
fn own_premises(
    im: &ItemImpl,
    is_edge: &dyn Fn(&ItemImpl, &WherePredicate) -> bool,
    share_self: bool,
) -> Vec<WherePredicate> {
    let mut singles = inline_bound_predicates(im);
    for pred in im
        .generics
        .where_clause
        .iter()
        .flat_map(|wc| &wc.predicates)
    {
        singles.extend(split(pred));
    }
    singles
        .into_iter()
        .filter(|p| !is_relaxed(p) && !is_edge(im, p))
        .filter(|p| share_self || !Names::of_predicate(p).idents.contains("Self"))
        .filter_map(|p| resolve_self(p, &im.self_ty))
        .collect()
}

/// `pred` with every `Self` that is a whole type replaced by `self_ty`; `None` if any other `Self`
/// remains.
fn resolve_self(mut pred: WherePredicate, self_ty: &Type) -> Option<WherePredicate> {
    struct R<'a>(&'a Type);
    impl VisitMut for R<'_> {
        fn visit_type_mut(&mut self, ty: &mut Type) {
            if let Type::Path(TypePath { qself: None, path }) = ty {
                if path.is_ident("Self") {
                    *ty = parenthesize(self.0.clone());
                    return;
                }
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
    }
    R(self_ty).visit_where_predicate_mut(&mut pred);
    (!Names::of_predicate(&pred).idents.contains("Self")).then_some(pred)
}

/// Would the ranked engine's registration check spell `q` (a premise of the impl with parameters
/// `params`) as [`translate`] does, mapped on through any substitution of type parameters? See
/// [`Respell::TypeParams`].
fn spelled_alike_by_ranked(q: &WherePredicate, params: &Params) -> bool {
    let WherePredicate::Type(pt) = q else {
        return true;
    };
    if !pt
        .bounds
        .iter()
        .any(|b| matches!(b, TypeParamBound::Trait(_)))
    {
        return true;
    }
    let names = Names::of_predicate(q);
    if pt.lifetimes.is_some()
        || names.lifetimes.iter().any(|l| l != "'static")
        || names.idents.iter().any(|i| params.consts.contains(i))
    {
        return false;
    }
    struct Projection<'a>(&'a Params, bool);
    impl Visit<'_> for Projection<'_> {
        fn visit_type_path(&mut self, tp: &TypePath) {
            if tp.qself.is_none()
                && tp.path.segments.len() > 1
                && self
                    .0
                    .names
                    .contains(&tp.path.segments[0].ident.to_string())
            {
                self.1 = true;
            }
            syn::visit::visit_type_path(self, tp);
        }
    }
    let mut projection = Projection(params, false);
    projection.visit_where_predicate(q);
    if projection.1 {
        return false;
    }
    let mentions = |f: &dyn Fn(&mut Names)| {
        let mut n = Names::default();
        f(&mut n);
        params.mentioned_by(&n)
    };
    pt.bounds.iter().all(|b| {
        let TypeParamBound::Trait(tb) = b else {
            return true;
        };
        tb.path.segments.iter().all(|seg| match &seg.arguments {
            PathArguments::None => true,
            PathArguments::Parenthesized(args) => {
                !mentions(&|n: &mut Names| n.visit_parenthesized_generic_arguments(args))
            }
            PathArguments::AngleBracketed(args) => args.args.iter().all(|a| match a {
                GenericArgument::Type(_) | GenericArgument::Lifetime(_) => true,
                other => !mentions(&|n: &mut Names| n.visit_generic_argument(other)),
            }),
        })
    })
}

// ----- matching an impl header against a bound ------------------------------------------------

/// Match `j`'s header against `edge`: the mapping of `j`'s parameters (`vars`) that makes its self
/// type the edge's target and its trait the edge's trait, if there is one. `receiver` are the
/// parameters of the impl the edge belongs to.
fn match_header(j: &ItemImpl, vars: &Params, edge: &CycleEdge, receiver: &Params) -> Option<Subst> {
    let mut m = Matcher::new(vars, receiver, Reading::Premises);
    let j_trait = &j.trait_.as_ref()?.1;
    (m.args(
        last_args_of_type(&j.self_ty)?,
        last_args_of_type(&edge.target)?,
    ) && m.args(
        &j_trait.segments.last()?.arguments,
        &edge.trait_path.segments.last()?.arguments,
    ))
    .then_some(m.subst)
}

fn last_args_of_type(ty: &Type) -> Option<&PathArguments> {
    match strip_parens(ty) {
        Type::Path(TypePath { qself: None, path }) => Some(&path.segments.last()?.arguments),
        _ => None,
    }
}

// ----- translating a premise ------------------------------------------------------------------

/// `pred`, a premise of the reached impl (parameters `vars`), in the terms of the impl reaching it
/// (parameters `receiver`) through a bound whose match is `subst` and whose binder is `binder` — or
/// `None` when it cannot be said there (see the module docs).
fn translate(
    pred: &WherePredicate,
    vars: &Params,
    subst: &Subst,
    binder: &[Lifetime],
    receiver: &Params,
) -> Option<WherePredicate> {
    let names = Names::of_predicate(pred);
    if names.opaque || names.idents.contains("Self") {
        return None;
    }
    for ident in &names.idents {
        if vars.names.contains(ident) {
            // A parameter the match leaves open has no counterpart.
            if !subst.contains_key(ident) {
                return None;
            }
        } else if receiver.names.contains(ident) {
            // A name of the reached impl's that the receiving impl declares as a parameter would
            // mean that parameter there.
            return None;
        }
    }
    for lt in &names.lifetimes {
        if names.bound.contains(lt) || lt == "'static" || lt == "'_" {
            continue;
        }
        if !(vars.lifetimes.contains(lt) && subst.contains_key(lt)) {
            return None;
        }
    }
    // The lifetimes the premise's own binders must stay apart from: the receiving impl's, and those
    // the substitution brings in (the edge's binder among them).
    let mut taken: HashSet<String> = receiver.lifetimes.clone();
    taken.extend(binder.iter().map(|l| l.to_string()));
    for name in names.idents.iter().chain(&names.lifetimes) {
        if let Some(v) = subst.get(name) {
            taken.extend(Names::of_value(v).lifetimes);
        }
    }
    let mut pred = pred.clone();
    let mut renames: HashMap<String, Lifetime> = HashMap::new();
    let mut avoid: HashSet<String> = taken.iter().chain(&names.lifetimes).cloned().collect();
    for b in &names.bound {
        if taken.contains(b) {
            let fresh = fresh_lifetime(b, &avoid);
            avoid.insert(fresh.to_string());
            renames.insert(b.clone(), fresh);
        }
    }
    if !renames.is_empty() {
        RenameLifetimes(&renames).visit_where_predicate_mut(&mut pred);
    }
    let mut apply = Apply::new(subst, vars, Reading::Premises);
    apply.visit_where_predicate_mut(&mut pred);
    if apply.failed {
        return None;
    }
    // A lifetime the edge's binder binds is one for every instantiation: bind it on the premise too.
    let now = Names::of_predicate(&pred);
    let used: Vec<&Lifetime> = binder
        .iter()
        .filter(|l| now.lifetimes.contains(&l.to_string()) && !now.bound.contains(&l.to_string()))
        .collect();
    if !used.is_empty() {
        let WherePredicate::Type(pt) = &mut pred else {
            return None;
        };
        let bl = pt.lifetimes.get_or_insert_with(BoundLifetimes::default);
        for l in used {
            bl.lifetimes
                .push(GenericParam::Lifetime(LifetimeParam::new(l.clone())));
        }
    }
    Some(pred)
}

fn fresh_lifetime(base: &str, avoid: &HashSet<String>) -> Lifetime {
    let stem = base.trim_start_matches('\'');
    let mut name = format!("'{stem}__decycle_hr");
    let mut n = 1;
    while avoid.contains(&name) {
        name = format!("'{stem}__decycle_hr{n}");
        n += 1;
    }
    Lifetime::new(&name, proc_macro2::Span::call_site())
}

/// Renames every lifetime the map has a key for (keyed by its spelling, `'a`).
pub(crate) struct RenameLifetimes<'a>(pub(crate) &'a HashMap<String, Lifetime>);

impl VisitMut for RenameLifetimes<'_> {
    fn visit_lifetime_mut(&mut self, lt: &mut Lifetime) {
        if let Some(new) = self.0.get(&lt.to_string()) {
            *lt = new.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    /// Run the pass over `impls`, reading a bound `X: Tr<..>` as an edge to every impl of `Tr` whose
    /// self type has `X`'s head, as both engines do; the premises each impl receives, spelled
    /// without whitespace.
    fn run(impls: &[ItemImpl], respell: Respell) -> Vec<Vec<String>> {
        let refs: Vec<&ItemImpl> = impls.iter().collect();
        let head = |ty: &Type| crate::names::local_type_head_ident(strip_parens(ty));
        let trait_of = |im: &ItemImpl| {
            im.trait_
                .as_ref()
                .map(|(_, p, _)| p.segments.last().unwrap().ident.clone())
        };
        let traits: HashSet<_> = refs.iter().filter_map(|im| trait_of(im)).collect();
        let heads: HashSet<_> = refs.iter().filter_map(|im| head(&im.self_ty)).collect();
        let is_edge = |_: &ItemImpl, p: &WherePredicate| match p {
            WherePredicate::Type(pt) => {
                head(&pt.bounded_ty).is_some_and(|h| heads.contains(&h))
                    && pt.bounds.iter().any(|b| {
                        matches!(b, TypeParamBound::Trait(tb)
                        if traits.contains(&tb.path.segments.last().unwrap().ident))
                    })
            }
            _ => false,
        };
        let edges: Vec<Vec<CycleEdge>> = refs
            .iter()
            .map(|im| {
                let mut out = Vec::new();
                for pt in crate::names::type_predicates(&im.generics) {
                    for b in &pt.bounds {
                        let TypeParamBound::Trait(tb) = b else {
                            continue;
                        };
                        let tr = &tb.path.segments.last().unwrap().ident;
                        let candidates: Vec<usize> = refs
                            .iter()
                            .enumerate()
                            .filter(|(_, j)| {
                                trait_of(j).as_ref() == Some(tr)
                                    && head(&j.self_ty).is_some()
                                    && head(&j.self_ty) == head(&pt.bounded_ty)
                            })
                            .map(|(k, _)| k)
                            .collect();
                        if candidates.is_empty() {
                            continue;
                        }
                        let binder = pt
                            .lifetimes
                            .iter()
                            .flat_map(|b| &b.lifetimes)
                            .filter_map(|p| match p {
                                GenericParam::Lifetime(l) => Some(l.lifetime.clone()),
                                _ => None,
                            })
                            .collect();
                        out.push(CycleEdge {
                            target: pt.bounded_ty.clone(),
                            trait_path: tb.path.clone(),
                            binder,
                            candidates,
                        });
                    }
                }
                out
            })
            .collect();
        mapped_premises(&refs, &edges, &is_edge, respell)
            .iter()
            .map(|ps| {
                ps.iter()
                    .map(|p| spelling(p).chars().filter(|c| !c.is_whitespace()).collect())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_premise_is_mapped_through_the_bound_not_by_name() {
        let got = run(
            &[
                parse_quote!(
                    impl<T> Eval<T> for Expr where Term: Size<T> {}
                ),
                parse_quote!(
                    impl<U> Size<U> for Term
                    where
                        Expr: Eval<U>,
                        U: Clone,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got, vec![vec!["T:Clone".to_string()], vec![]]);
    }

    #[test]
    fn a_premise_follows_the_instantiation_the_bound_names() {
        // `Expr` reaches `Term` at `u8` only: `Term`'s `T: Clone` is `u8: Clone` there, never
        // `Expr`'s `T: Clone`; `Term` reaches `Expr` at its own `T`, so it needs `T: Default`.
        let got = run(
            &[
                parse_quote!(
                    impl<T> Eval<T> for Expr
                    where
                        Term: Size<u8>,
                        T: Default,
                    {
                    }
                ),
                parse_quote!(
                    impl<T> Size<T> for Term
                    where
                        Expr: Eval<T>,
                        T: Clone,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got[0], vec!["u8:Clone", "u8:Default"]);
        assert!(got[1].contains(&"T:Default".to_string()), "{:?}", got[1]);
        assert!(!got[1].contains(&"T:Clone".to_string()), "{:?}", got[1]);
    }

    #[test]
    fn swapped_parameters_are_swapped_back() {
        let got = run(
            &[
                parse_quote!(
                    impl<T, U> Tr<T, U> for A where B: Tr<U, T> {}
                ),
                parse_quote!(
                    impl<T, U> Tr<T, U> for B
                    where
                        A: Tr<U, T>,
                        T: Clone,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got, vec![vec!["U:Clone".to_string()], vec![]]);
    }

    #[test]
    fn transitively_across_three_traits() {
        let got = run(
            &[
                parse_quote!(
                    impl<T> Eval<T> for A
                    where
                        B: Size<T>,
                        T: PartialEq,
                    {
                    }
                ),
                parse_quote!(
                    impl<U> Size<U> for B
                    where
                        C: Show<U>,
                        U: Default,
                    {
                    }
                ),
                parse_quote!(
                    impl<V> Show<V> for C
                    where
                        A: Eval<V>,
                        V: Clone,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got[0], vec!["T:Default", "T:Clone"]);
        assert_eq!(got[1], vec!["U:Clone", "U:PartialEq"]);
        assert_eq!(got[2], vec!["V:PartialEq", "V:Default"]);
    }

    #[test]
    fn a_parameter_with_no_counterpart_is_not_shared() {
        // `'u` is fixed by no argument of the bound; its co-bound `T: Clone` is shared on its own.
        let got = run(
            &[
                parse_quote!(
                    impl<T> Eval<T> for Expr where Term: Size<T> {}
                ),
                parse_quote!(
                    impl<'u, T> Size<T> for Term
                    where
                        Expr: Eval<T>,
                        T: Clone + 'u,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got, vec![vec!["T:Clone".to_string()], vec![]]);
    }

    #[test]
    fn a_name_the_receiver_shadows_is_not_shared() {
        // `Foo` is an item on `Term`'s side and a parameter on `Expr`'s.
        let got = run(
            &[
                parse_quote!(
                    impl<Foo> Eval<Foo> for Expr where Term: Size<Foo> {}
                ),
                parse_quote!(
                    impl<T> Size<T> for Term
                    where
                        Expr: Eval<T>,
                        T: From<Foo>,
                        T: Clone,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got, vec![vec!["Foo:Clone".to_string()], vec![]]);
    }

    #[test]
    fn a_header_that_does_not_match_is_not_reached() {
        let got = run(
            &[
                parse_quote!(
                    impl<T> Eval<T> for Expr where Term: Size<T> {}
                ),
                parse_quote!(impl Size<u8> for Term where Expr: Eval<u8>, u8: Clone {}),
            ],
            Respell::Any,
        );
        assert!(got[0].is_empty(), "{:?}", got[0]);
    }

    #[test]
    fn lifetimes_map_by_position_and_binders_stay_apart() {
        let got = run(
            &[
                parse_quote!(
                    impl<'s, 'x, T> Eval<'x, T> for Expr<'s> where Term<'s>: Size<'x, T> {}
                ),
                parse_quote!(
                    impl<'t, 'b, U> Size<'b, U> for Term<'t>
                    where
                        Expr<'t>: Eval<'b, U>,
                        U: 'b,
                        't: 'b,
                        for<'x> &'x U: Debug,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        assert_eq!(
            got[0],
            vec!["T:'x", "'s:'x", "for<'x__decycle_hr>&'x__decycle_hrT:Debug"]
        );
        assert!(got[1].is_empty(), "{:?}", got[1]);
    }

    #[test]
    fn the_ranked_spelling_shares_what_its_check_reads_alike() {
        // A trait bound naming a renamed lifetime or binder is left out; lifetime bounds, which
        // the check never compares, and plain type parameters are shared.
        let got = run(
            &[
                parse_quote!(
                    impl<'s, 'x, T> Eval<'x, T> for Expr<'s> where Term<'s>: Size<'x, T> {}
                ),
                parse_quote!(
                    impl<'t, 'b, U> Size<'b, U> for Term<'t>
                    where
                        Expr<'t>: Eval<'b, U>,
                        U: 'b + Clone,
                        't: 'b,
                        for<'x> &'x U: Debug,
                        &'t U: Debug,
                        U: Fn(Vec<U>),
                        U::Out: Copy,
                    {
                    }
                ),
            ],
            Respell::TypeParams,
        );
        assert_eq!(got[0], vec!["T:'x", "T:Clone", "'s:'x"]);
    }

    #[test]
    fn a_lifetime_the_bound_binds_is_bound_on_the_premise() {
        let got = run(
            &[
                parse_quote!(
                    impl<T> Eval<T> for Expr where for<'a> Term<'a>: Size<T> {}
                ),
                parse_quote!(
                    impl<'s, T> Size<T> for Term<'s>
                    where
                        Expr: Eval<T>,
                        T: 's,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got[0], vec!["for<'a>T:'a"]);
    }

    #[test]
    fn inline_bounds_and_co_bounds_are_premises() {
        let got = run(
            &[
                parse_quote!(
                    impl<T> Eval<T> for Expr where Term: Size<T> {}
                ),
                parse_quote!(
                    impl<U: Clone> Size<U> for Term where Expr: Eval<U> + Debug {}
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got[0], vec!["T:Clone", "Expr:Debug"]);
    }

    #[test]
    fn self_is_shared_as_the_reached_self_type_on_request() {
        let impls: [ItemImpl; 2] = [
            parse_quote!(
                impl<S, T> Eval<T> for Expr<S> where Term<S>: Size<T> {}
            ),
            parse_quote!(
                impl<R, U> Size<U> for Term<R>
                where
                    Expr<R>: Eval<U>,
                    Self: Debug,
                    R: Copy,
                {
                }
            ),
        ];
        assert_eq!(
            run(&impls, Respell::Any)[0],
            vec!["Term<S>:Debug", "S:Copy"]
        );
        assert_eq!(run(&impls, Respell::TypeParams)[0], vec!["S:Copy"]);
    }

    #[test]
    fn const_parameters_map_too() {
        let got = run(
            &[
                parse_quote!(
                    impl<const N: usize> Eval<N> for Expr
                    where
                        Term: Size<N>,
                        Lit: Size<3>,
                    {
                    }
                ),
                parse_quote!(
                    impl<const M: usize> Size<M> for Term
                    where
                        Expr: Eval<M>,
                        [u8; M]: Default,
                    {
                    }
                ),
                parse_quote!(
                    impl<const M: usize> Size<M> for Lit where [u16; M]: Default {}
                ),
            ],
            Respell::Any,
        );
        assert_eq!(got[0], vec!["[u8;N]:Default", "[u16;3]:Default"]);
    }

    #[test]
    fn a_premise_on_an_inline_bounded_parameter_joins_its_bounds() {
        let mut im: ItemImpl = parse_quote!(
            impl<'a: 'static, 'b, T: Clone, U> Tr<'b, T, U> for A<'a> where U: Copy {}
        );
        let add = vec![
            parse_quote!(T: Default),
            parse_quote!(U: Debug),
            parse_quote!(T: Clone),
            parse_quote!('a: 'b),
            parse_quote!(for<'x> &'x T: Debug),
        ];
        assert!(add_premises(&mut im, add));
        let got: String = spelling(&im).split_whitespace().collect();
        assert_eq!(
            got,
            "impl<'a:'static+'b,'b,T:Clone+Default,U>Tr<'b,T,U>forA<'a>\
             whereU:Copy,U:Debug,for<'x>&'xT:Debug{}"
        );
        assert!(!add_premises(&mut im, vec![parse_quote!(T: Default)]));
    }

    #[test]
    fn a_growing_cycle_stops() {
        let got = run(
            &[
                parse_quote!(
                    impl<T> Eval<T> for Expr where Term: Size<Vec<T>> {}
                ),
                parse_quote!(
                    impl<T> Size<T> for Term
                    where
                        Expr: Eval<T>,
                        T: Clone,
                    {
                    }
                ),
            ],
            Respell::Any,
        );
        // Every lap wraps one more `Vec`; the walk gives up after `2n + 2` bounds.
        assert!(got[0].len() <= 6, "{:?}", got[0]);
        assert_eq!(got[0][0], "Vec<T>:Clone");
    }
}
