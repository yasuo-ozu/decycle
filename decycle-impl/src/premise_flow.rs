//! Carry the premises of a cycle's impls along its cyclic bounds, whatever their trait, with the
//! generic parameters mapped through each bound — both engines.
//!
//! An impl's generated code reaches each impl its cyclic bounds name through the public trait, and
//! that impl demands its own premises. Plain Rust proves them through the cyclic bound itself; the
//! engines strip (structural) or rank (ranked) that bound, so the impl needs them in its own
//! environment. [`crate::ranked::sharing`] gives the impls of one trait each other's premises by
//! *name*; that never reaches an impl of another trait in a cycle that alternates between traits
//! (`Expr: Eval` needing `Term: Size`), and it cannot map a parameter that is named differently on
//! the two sides. This pass follows the bounds instead:
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
    BoundLifetimes, Expr, GenericArgument, GenericParam, ItemImpl, Lifetime, LifetimeParam, Path,
    PathArguments, PredicateLifetime, PredicateType, Token, TraitBoundModifier, Type,
    TypeParamBound, TypePath, WherePredicate,
};
use template_quote::ToTokens;

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

/// How the impl receiving a premise may spell it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Respell {
    /// In whatever way says the same thing in its terms: lifetimes renamed, a binder renamed apart
    /// or added, `Self` spelled as the reached impl's self type (the structural engine).
    Any,
    /// Only as the ranked engine's re-entry registration check (`reachable_side_bounds_ok`) spells
    /// it. That check maps a reached impl's premises back into the terms of the impl reaching it by
    /// substituting type parameters (in the bounded type and in the bound's `<..>` type arguments)
    /// and compares them by spelling; a premise it would spell differently from this pass would not
    /// be recognised there, and the registration of an impl that registered before would now be
    /// skipped. So a premise with a trait bound is only shared when the two spellings agree under
    /// any substitution: no lifetime but `'static` and no `for<..>`, no const parameter, no
    /// projection on a parameter (`T::Out`), no parameter in `Fn(..)` sugar, an associated-item
    /// constraint or a const argument of the bound; and a premise naming `Self` is not shared at
    /// all (the check reads `Self` as written: `Self: Debug` would not recognise `Term<R>: Debug`).
    /// A premise with lifetime bounds only (`T: 'a`, `'a: 'b`) is never compared, and is shared.
    TypeParams,
}

/// The premises each impl of `impls` receives along `edges` (`edges[i]` the cyclic bounds of
/// `impls[i]`), in its own terms, without the ones it states already. `is_edge` tells a cyclic bound
/// from a premise, as the engine reads them; neither a cyclic bound nor a premise that would read as
/// one in the receiving impl is shared.
pub(crate) fn mapped_premises(
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
pub(crate) fn add_premises(im: &mut ItemImpl, premises: Vec<WherePredicate>) -> bool {
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

fn spelling(p: &impl ToTokens) -> String {
    p.to_token_stream().to_string()
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

/// An impl's generic parameters: `names` its type and const parameters, `lifetimes` its lifetime
/// parameters (spelled `'a`).
struct Params {
    names: HashSet<String>,
    consts: HashSet<String>,
    lifetimes: HashSet<String>,
}

impl Params {
    fn of(im: &ItemImpl) -> Self {
        let mut p = Params {
            names: HashSet::new(),
            consts: HashSet::new(),
            lifetimes: HashSet::new(),
        };
        for param in &im.generics.params {
            match param {
                GenericParam::Type(t) => {
                    p.names.insert(t.ident.to_string());
                }
                GenericParam::Const(c) => {
                    p.names.insert(c.ident.to_string());
                    p.consts.insert(c.ident.to_string());
                }
                GenericParam::Lifetime(l) => {
                    p.lifetimes.insert(l.lifetime.to_string());
                }
            }
        }
        p
    }

    /// The parameter a type spelled as one bare name is, if it is one.
    fn type_var(&self, ty: &Type) -> Option<String> {
        let Type::Path(TypePath { qself: None, path }) = ty else {
            return None;
        };
        let ident = path.get_ident()?.to_string();
        self.names.contains(&ident).then_some(ident)
    }

    fn mentioned_by(&self, names: &Names) -> bool {
        names.idents.iter().any(|i| self.names.contains(i))
            || names.lifetimes.iter().any(|l| self.lifetimes.contains(l))
    }
}

/// What a parameter of the reached impl stands for in the impl reaching it.
#[derive(Clone)]
enum Value {
    Type(Type),
    Const(Expr),
    Lifetime(Lifetime),
}

impl Value {
    fn render(&self) -> String {
        match self {
            Value::Type(t) => spelling(t),
            Value::Const(e) => spelling(e),
            Value::Lifetime(l) => spelling(l),
        }
    }
}

/// A mapping from a reached impl's parameters (by name; a lifetime as `'a`) to what they stand for.
type Subst = HashMap<String, Value>;

/// The names a syntax node mentions.
#[derive(Default)]
struct Names {
    /// Every ident (types, consts, path segments, traits) but a lifetime's.
    idents: HashSet<String>,
    /// Every lifetime, `'a`.
    lifetimes: HashSet<String>,
    /// The lifetimes a `for<..>` inside the node binds.
    bound: HashSet<String>,
    /// The node holds tokens this visitor cannot see into (a macro, verbatim tokens).
    opaque: bool,
}

impl Names {
    fn of_predicate(p: &WherePredicate) -> Self {
        let mut n = Names::default();
        n.visit_where_predicate(p);
        n
    }
    fn of_type(t: &Type) -> Self {
        let mut n = Names::default();
        n.visit_type(t);
        n
    }
    fn of_expr(e: &Expr) -> Self {
        let mut n = Names::default();
        n.visit_expr(e);
        n
    }
    fn of_value(v: &Value) -> Self {
        match v {
            Value::Type(t) => Names::of_type(t),
            Value::Const(e) => Names::of_expr(e),
            Value::Lifetime(l) => {
                let mut n = Names::default();
                n.visit_lifetime(l);
                n
            }
        }
    }
}

impl Visit<'_> for Names {
    fn visit_ident(&mut self, i: &proc_macro2::Ident) {
        self.idents.insert(i.to_string());
    }
    // Not recursing: the default would report the lifetime's ident as an ident.
    fn visit_lifetime(&mut self, lt: &Lifetime) {
        self.lifetimes.insert(lt.to_string());
    }
    fn visit_bound_lifetimes(&mut self, b: &BoundLifetimes) {
        for p in &b.lifetimes {
            if let GenericParam::Lifetime(l) = p {
                self.bound.insert(l.lifetime.to_string());
            }
        }
        syn::visit::visit_bound_lifetimes(self, b);
    }
    fn visit_macro(&mut self, m: &syn::Macro) {
        self.opaque = true;
        syn::visit::visit_macro(self, m);
    }
    fn visit_type(&mut self, t: &Type) {
        if let Type::Verbatim(_) = t {
            self.opaque = true;
        }
        syn::visit::visit_type(self, t);
    }
    fn visit_expr(&mut self, e: &Expr) {
        if let Expr::Verbatim(_) = e {
            self.opaque = true;
        }
        syn::visit::visit_expr(self, e);
    }
    fn visit_type_param_bound(&mut self, b: &TypeParamBound) {
        if let TypeParamBound::Verbatim(_) = b {
            self.opaque = true;
        }
        syn::visit::visit_type_param_bound(self, b);
    }
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

/// `ty`, parenthesized if it could not otherwise stand where a type parameter stood (`&'a T` with
/// `T` = `dyn A + B`).
fn parenthesize(ty: Type) -> Type {
    match ty {
        Type::TraitObject(_) | Type::ImplTrait(_) => Type::Paren(syn::TypeParen {
            paren_token: Default::default(),
            elem: Box::new(ty),
        }),
        other => other,
    }
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
    let mut m = Matcher {
        vars,
        receiver,
        subst: Subst::new(),
    };
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
    match strip(ty) {
        Type::Path(TypePath { qself: None, path }) => Some(&path.segments.last()?.arguments),
        _ => None,
    }
}

/// See through `(T)` and invisible groups.
fn strip(ty: &Type) -> &Type {
    match ty {
        Type::Paren(p) => strip(&p.elem),
        Type::Group(g) => strip(&g.elem),
        other => other,
    }
}

/// First-order matching of a pattern (the reached impl's header, its parameters the variables)
/// against the bound that reaches it (written in the receiving impl's terms). Fails on anything it
/// cannot establish syntactically, which then reads as "this impl is not reached".
struct Matcher<'a> {
    vars: &'a Params,
    receiver: &'a Params,
    subst: Subst,
}

impl Matcher<'_> {
    fn bind(&mut self, var: String, value: Value) -> bool {
        match self.subst.get(&var) {
            Some(old) => old.render() == value.render(),
            None => {
                self.subst.insert(var, value);
                true
            }
        }
    }

    fn ty(&mut self, p: &Type, t: &Type) -> bool {
        let (p, t) = (strip(p), strip(t));
        if let Some(var) = self.vars.type_var(p) {
            return self.bind(var, Value::Type(t.clone()));
        }
        match (p, t) {
            (Type::Path(pp), Type::Path(tp)) if pp.qself.is_none() && tp.qself.is_none() => {
                self.path(&pp.path, &tp.path)
            }
            (Type::Reference(pr), Type::Reference(tr)) => {
                pr.mutability.is_some() == tr.mutability.is_some()
                    && self.opt_lifetime(pr.lifetime.as_ref(), tr.lifetime.as_ref())
                    && self.ty(&pr.elem, &tr.elem)
            }
            (Type::Ptr(pp), Type::Ptr(tp)) => {
                pp.mutability.is_some() == tp.mutability.is_some() && self.ty(&pp.elem, &tp.elem)
            }
            (Type::Slice(ps), Type::Slice(ts)) => self.ty(&ps.elem, &ts.elem),
            (Type::Array(pa), Type::Array(ta)) => {
                self.ty(&pa.elem, &ta.elem) && self.expr(&pa.len, &ta.len)
            }
            (Type::Tuple(pt), Type::Tuple(tt)) => {
                pt.elems.len() == tt.elems.len()
                    && pt.elems.iter().zip(&tt.elems).all(|(a, b)| self.ty(a, b))
            }
            _ => self.opaque(
                &Names::of_type(p),
                &Names::of_type(t),
                spelling(p) == spelling(t),
            ),
        }
    }

    fn path(&mut self, p: &Path, t: &Path) -> bool {
        // `self::Foo` is `Foo`.
        let (mut p, mut t) = (p.clone(), t.clone());
        crate::helper::strip_leading_self(&mut p);
        crate::helper::strip_leading_self(&mut t);
        if p.leading_colon.is_some() != t.leading_colon.is_some()
            || p.segments.len() != t.segments.len()
        {
            return false;
        }
        // A variable that is not the whole type (`T::Out`) cannot be matched by its shape.
        if p.segments
            .first()
            .is_some_and(|s| self.vars.names.contains(&s.ident.to_string()))
        {
            return false;
        }
        // The pattern names a concrete item here; a target that is a parameter of the receiving impl
        // spelled the same is something else.
        if t.get_ident()
            .is_some_and(|i| self.receiver.names.contains(&i.to_string()))
        {
            return false;
        }
        p.segments
            .iter()
            .zip(&t.segments)
            .all(|(ps, ts)| ps.ident == ts.ident && self.args(&ps.arguments, &ts.arguments))
    }

    fn args(&mut self, p: &PathArguments, t: &PathArguments) -> bool {
        match (p, t) {
            (PathArguments::None, PathArguments::None) => true,
            (PathArguments::AngleBracketed(pa), PathArguments::AngleBracketed(ta)) => {
                self.arg_list(&pa.args, &ta.args)
            }
            (PathArguments::None, PathArguments::AngleBracketed(ta)) => {
                self.arg_list(&Punctuated::new(), &ta.args)
            }
            (PathArguments::AngleBracketed(pa), PathArguments::None) => {
                self.arg_list(&pa.args, &Punctuated::new())
            }
            (PathArguments::Parenthesized(pa), PathArguments::Parenthesized(ta)) => {
                pa.inputs.len() == ta.inputs.len()
                    && pa.inputs.iter().zip(&ta.inputs).all(|(a, b)| self.ty(a, b))
                    && match (&pa.output, &ta.output) {
                        (syn::ReturnType::Default, syn::ReturnType::Default) => true,
                        (syn::ReturnType::Type(_, a), syn::ReturnType::Type(_, b)) => self.ty(a, b),
                        _ => false,
                    }
            }
            _ => false,
        }
    }

    /// Arguments by kind: lifetimes pairwise (unless the target elides them all), and the type and
    /// const arguments pairwise. An associated-item constraint of the target (`Tr<T, Out = u8>`)
    /// selects no impl and is ignored; one in the pattern is not understood.
    fn arg_list(
        &mut self,
        p: &Punctuated<GenericArgument, Token![,]>,
        t: &Punctuated<GenericArgument, Token![,]>,
    ) -> bool {
        let mut p_lts = Vec::new();
        let mut p_rest = Vec::new();
        for a in p {
            match a {
                GenericArgument::Lifetime(l) => p_lts.push(l),
                GenericArgument::Type(_) | GenericArgument::Const(_) => p_rest.push(a),
                _ => return false,
            }
        }
        let mut t_lts = Vec::new();
        let mut t_rest = Vec::new();
        for a in t {
            match a {
                GenericArgument::Lifetime(l) => t_lts.push(l),
                GenericArgument::Type(_) | GenericArgument::Const(_) => t_rest.push(a),
                _ => {}
            }
        }
        // A target that elides its lifetimes fixes none of the pattern's.
        if !t_lts.is_empty()
            && (p_lts.len() != t_lts.len()
                || !p_lts.iter().zip(&t_lts).all(|(a, b)| self.lifetime(a, b)))
        {
            return false;
        }
        p_rest.len() == t_rest.len() && p_rest.iter().zip(&t_rest).all(|(a, b)| self.arg(a, b))
    }

    fn arg(&mut self, p: &GenericArgument, t: &GenericArgument) -> bool {
        match (p, t) {
            (GenericArgument::Type(pt), GenericArgument::Type(tt)) => self.ty(pt, tt),
            // A const parameter, spelled as a bare name, against a const expression.
            (GenericArgument::Type(pt), GenericArgument::Const(te)) => match self.vars.type_var(pt)
            {
                Some(var) if self.vars.consts.contains(&var) => {
                    self.bind(var, Value::Const(te.clone()))
                }
                _ => false,
            },
            (GenericArgument::Const(pe), GenericArgument::Const(te)) => self.expr(pe, te),
            _ => false,
        }
    }

    fn expr(&mut self, p: &Expr, t: &Expr) -> bool {
        if let Expr::Path(ep) = p {
            if ep.qself.is_none() {
                if let Some(ident) = ep.path.get_ident() {
                    let var = ident.to_string();
                    if self.vars.consts.contains(&var) {
                        return self.bind(var, Value::Const(t.clone()));
                    }
                }
            }
        }
        self.opaque(
            &Names::of_expr(p),
            &Names::of_expr(t),
            spelling(p) == spelling(t),
        )
    }

    fn lifetime(&mut self, p: &Lifetime, t: &Lifetime) -> bool {
        let var = p.to_string();
        if self.vars.lifetimes.contains(&var) {
            return self.bind(var, Value::Lifetime(t.clone()));
        }
        // `'_` in an impl header is a fresh lifetime of its own: anything matches it.
        p.ident == "_" || (var == t.to_string() && !self.receiver.lifetimes.contains(&var))
    }

    fn opt_lifetime(&mut self, p: Option<&Lifetime>, t: Option<&Lifetime>) -> bool {
        match (p, t) {
            (Some(p), Some(t)) => self.lifetime(p, t),
            (None, _) => true,
            // The target elides it: nothing to bind, but a named lifetime of the pattern's own
            // (`'static`) cannot be told to match.
            (Some(p), None) => self.vars.lifetimes.contains(&p.to_string()) || p.ident == "_",
        }
    }

    /// Two nodes this matcher does not take apart: they match when they are spelled the same and
    /// neither names a parameter (of the pattern's impl, which would need binding, or of the
    /// receiving impl, which the same spelling in the pattern does not mean).
    fn opaque(&self, p: &Names, t: &Names, same: bool) -> bool {
        same && !p.opaque
            && !t.opaque
            && !self.vars.mentioned_by(p)
            && !self.receiver.mentioned_by(t)
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
    let mut apply = Apply {
        subst,
        vars,
        failed: false,
    };
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

struct RenameLifetimes<'a>(&'a HashMap<String, Lifetime>);

impl VisitMut for RenameLifetimes<'_> {
    fn visit_lifetime_mut(&mut self, lt: &mut Lifetime) {
        if let Some(new) = self.0.get(&lt.to_string()) {
            *lt = new.clone();
        }
    }
}

/// Replace the reached impl's parameters by what they stand for.
struct Apply<'a> {
    subst: &'a Subst,
    vars: &'a Params,
    failed: bool,
}

impl Apply<'_> {
    /// What the parameter `ident` maps to, when it is one.
    fn value(&mut self, ident: &proc_macro2::Ident) -> Option<&Value> {
        let name = ident.to_string();
        if !self.vars.names.contains(&name) {
            return None;
        }
        let v = self.subst.get(&name);
        if v.is_none() {
            self.failed = true;
        }
        v
    }
}

/// The ident a value is, when it is spelled as one plain name.
fn plain_name(v: &Value) -> Option<proc_macro2::Ident> {
    match v {
        Value::Type(Type::Path(TypePath { qself: None, path })) => path.get_ident().cloned(),
        Value::Const(Expr::Path(ep)) if ep.qself.is_none() => ep.path.get_ident().cloned(),
        _ => None,
    }
}

impl VisitMut for Apply<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if let Type::Path(TypePath { qself: None, path }) = ty {
            if path.leading_colon.is_none() {
                if let Some(first) = path.segments.first() {
                    let whole = path.segments.len() == 1 && first.arguments.is_none();
                    let ident = first.ident.clone();
                    match self.value(&ident).cloned() {
                        Some(Value::Type(t)) if whole => {
                            *ty = parenthesize(t);
                            return;
                        }
                        // `T::Out`: only when `T` maps to a plain name.
                        Some(v) if !whole => match plain_name(&v) {
                            Some(name) => path.segments[0].ident = name,
                            None => {
                                self.failed = true;
                                return;
                            }
                        },
                        // A const parameter in a type position is a generic argument, handled by
                        // `visit_generic_argument_mut`; anything else cannot be spelled here.
                        Some(_) => {
                            self.failed = true;
                            return;
                        }
                        None => {}
                    }
                }
            }
        }
        syn::visit_mut::visit_type_mut(self, ty);
    }

    fn visit_generic_argument_mut(&mut self, arg: &mut GenericArgument) {
        if let GenericArgument::Type(Type::Path(TypePath { qself: None, path })) = arg {
            if let Some(ident) = path.get_ident().cloned() {
                if let Some(Value::Const(e)) = self.value(&ident).cloned() {
                    *arg = GenericArgument::Const(match e {
                        Expr::Lit(_) | Expr::Block(_) => e,
                        other => syn::parse_quote!({ #other }),
                    });
                    return;
                }
            }
        }
        syn::visit_mut::visit_generic_argument_mut(self, arg);
    }

    fn visit_expr_mut(&mut self, e: &mut Expr) {
        if let Expr::Path(ep) = e {
            if ep.qself.is_none() && ep.path.leading_colon.is_none() {
                if let Some(first) = ep.path.segments.first() {
                    let whole = ep.path.segments.len() == 1 && first.arguments.is_none();
                    let ident = first.ident.clone();
                    match self.value(&ident).cloned() {
                        Some(Value::Const(c)) if whole => {
                            *e = match c {
                                Expr::Lit(_) | Expr::Path(_) | Expr::Block(_) | Expr::Paren(_) => c,
                                other => syn::parse_quote!((#other)),
                            };
                            return;
                        }
                        Some(v) => match plain_name(&v) {
                            Some(name) => ep.path.segments[0].ident = name,
                            None => {
                                self.failed = true;
                                return;
                            }
                        },
                        None => {}
                    }
                }
            }
        }
        syn::visit_mut::visit_expr_mut(self, e);
    }

    fn visit_lifetime_mut(&mut self, lt: &mut Lifetime) {
        let name = lt.to_string();
        if !self.vars.lifetimes.contains(&name) {
            return;
        }
        match self.subst.get(&name) {
            Some(Value::Lifetime(l)) => *lt = l.clone(),
            _ => self.failed = true,
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
        let head = |ty: &Type| crate::helper::local_type_head_ident(strip(ty));
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
                for pred in im
                    .generics
                    .where_clause
                    .iter()
                    .flat_map(|wc| &wc.predicates)
                {
                    let WherePredicate::Type(pt) = pred else {
                        continue;
                    };
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
