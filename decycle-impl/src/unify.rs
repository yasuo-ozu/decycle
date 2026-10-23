//! One first-order matcher of an impl header against what an impl is asked to prove, and the
//! substitution of the impl's parameters it yields — shared by both readers of a cycle's impls:
//! [`crate::premise_flow`], which carries premises along a cycle's bounds (both engines), and the
//! ranked engine's re-entry registration check (`ranked::finalize::reachable_side_bounds_ok`).
//!
//! The pattern is the header (its impl's generic parameters the variables, [`Params`]); the target
//! is a bound, or an obligation, written in another impl's terms. A match is the mapping
//! ([`Subst`]) that makes the pattern the target; [`Apply`] then says the reached impl's premises
//! in those terms.
//!
//! The two readers do not read a header alike, and must not ([`Reading`]): the premises carried
//! along a bound are spelled into a `where`-clause, so lifetimes and const parameters are mapped
//! like types, and a node is only matched when it is understood; the registration check compares
//! what it derives with the registering impl's own bounds by spelling, through a registry that
//! erases lifetimes, so it ignores them and compares what it does not take apart by spelling.

use std::collections::HashSet;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;
use syn::{
    BoundLifetimes, Expr, GenericArgument, GenericParam, Ident, ItemImpl, Lifetime, Path,
    PathArguments, Token, Type, TypeParamBound, TypePath, WherePredicate,
};
use template_quote::ToTokens;

use crate::names::strip_parens;

pub(crate) fn spelling(p: &impl ToTokens) -> String {
    p.to_token_stream().to_string()
}

/// How [`Matcher`] reads a header against a target, and how [`Apply`] substitutes the match.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reading {
    /// [`crate::premise_flow`]'s: every generic parameter is a variable (a lifetime spelled `'a`);
    /// `(T)`, invisible groups and a no-op `self::` are seen through; a target that elides its
    /// arguments fixes none of the pattern's; references, pointers, slices, arrays and tuples are
    /// taken apart, and any other node matches only spelled alike and naming no parameter of
    /// either side. A parameter of the receiving impl spelled like a concrete item of the pattern
    /// is that parameter, not the item. The substitution parenthesizes a type that could not
    /// stand where the parameter stood, and maps `T::Out` through a plain name.
    Premises,
    /// The ranked engine's registration check's: only type parameters are variables; lifetimes
    /// are ignored (the re-entry registry erases them); a path is compared as written, argument
    /// for argument (a const argument by spelling); references, tuples, parentheses and qualified
    /// paths `<T as Tr>::X` are taken apart, and any other node matches spelled alike, whatever it
    /// names. The substitution replaces a parameter only where it is the whole type, as written.
    Registration,
}

/// An impl's generic parameters: `names` its type and const parameters, `lifetimes` its lifetime
/// parameters (spelled `'a`).
pub(crate) struct Params {
    pub(crate) names: HashSet<String>,
    pub(crate) consts: HashSet<String>,
    pub(crate) lifetimes: HashSet<String>,
}

impl Params {
    pub(crate) fn of(im: &ItemImpl) -> Self {
        let mut p = Params::none();
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

    fn none() -> Self {
        Params {
            names: HashSet::new(),
            consts: HashSet::new(),
            lifetimes: HashSet::new(),
        }
    }

    /// The type parameters `idents`, as the only variables ([`Reading::Registration`]).
    fn types(idents: &HashSet<Ident>) -> Self {
        let mut p = Params::none();
        p.names = idents.iter().map(|i| i.to_string()).collect();
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

    pub(crate) fn mentioned_by(&self, names: &Names) -> bool {
        names.idents.iter().any(|i| self.names.contains(i))
            || names.lifetimes.iter().any(|l| self.lifetimes.contains(l))
    }
}

/// What a parameter of the reached impl stands for in the impl reaching it.
#[derive(Clone)]
pub(crate) enum Value {
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
pub(crate) type Subst = std::collections::HashMap<String, Value>;

/// The names a syntax node mentions.
#[derive(Default)]
pub(crate) struct Names {
    /// Every ident (types, consts, path segments, traits) but a lifetime's.
    pub(crate) idents: HashSet<String>,
    /// Every lifetime, `'a`.
    pub(crate) lifetimes: HashSet<String>,
    /// The lifetimes a `for<..>` inside the node binds.
    pub(crate) bound: HashSet<String>,
    /// The node holds tokens this visitor cannot see into (a macro, verbatim tokens).
    pub(crate) opaque: bool,
}

impl Names {
    pub(crate) fn of_predicate(p: &WherePredicate) -> Self {
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
    pub(crate) fn of_value(v: &Value) -> Self {
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

// ----- the registration check's entry points ----------------------------------------------------

/// The match of `pattern` (with the type parameters `vars` free) against `concrete`, as the
/// registration check reads it ([`Reading::Registration`]).
pub(crate) fn unify_type(vars: &HashSet<Ident>, pattern: &Type, concrete: &Type) -> Option<Subst> {
    let (vars, receiver) = (Params::types(vars), Params::none());
    let mut m = Matcher::new(&vars, &receiver, Reading::Registration);
    m.ty(pattern, concrete).then_some(m.subst)
}

/// [`unify_type`] of a candidate impl against an obligation, matching BOTH its self type and its
/// trait arguments (`impl<T> Cast<T> for A` against `A: Cast<i64>` binds `T = i64`). A trait's own
/// type parameter is determined by the trait argument, not the self type, so unifying the self
/// type alone left such a `T` forever unbound and wrongly rejected the whole registration.
pub(crate) fn unify_obligation(
    vars: &HashSet<Ident>,
    cand_self: &Type,
    cand_targs: &[GenericArgument],
    obl_self: &Type,
    obl_targs: &[GenericArgument],
) -> Option<Subst> {
    let (vars, receiver) = (Params::types(vars), Params::none());
    let mut m = Matcher::new(&vars, &receiver, Reading::Registration);
    let (cand_targs, obl_targs): (Vec<_>, Vec<_>) =
        (cand_targs.iter().collect(), obl_targs.iter().collect());
    (m.ty(cand_self, obl_self) && m.positional(&cand_targs, &obl_targs)).then_some(m.subst)
}

/// `ty` with the match `subst` applied as the registration check reads it: every parameter that is
/// a whole type, as written, replaced by what it stands for.
pub(crate) fn substitute(ty: &Type, subst: &Subst) -> Type {
    let vars = Params::none();
    let mut apply = Apply::new(subst, &vars, Reading::Registration);
    let mut t = ty.clone();
    apply.visit_type_mut(&mut t);
    t
}

// ----- matching ---------------------------------------------------------------------------------

/// First-order matching of a pattern (an impl's header, its parameters `vars` the variables)
/// against a target written in the receiving impl's terms (its parameters `receiver`). Fails on
/// anything it cannot establish syntactically, which then reads as "this impl is not reached".
pub(crate) struct Matcher<'a> {
    vars: &'a Params,
    receiver: &'a Params,
    reading: Reading,
    pub(crate) subst: Subst,
}

impl<'a> Matcher<'a> {
    pub(crate) fn new(vars: &'a Params, receiver: &'a Params, reading: Reading) -> Self {
        Matcher {
            vars,
            receiver,
            reading,
            subst: Subst::new(),
        }
    }

    fn bind(&mut self, var: String, value: Value) -> bool {
        match self.subst.get(&var) {
            Some(old) => old.render() == value.render(),
            None => {
                self.subst.insert(var, value);
                true
            }
        }
    }

    pub(crate) fn ty(&mut self, p: &Type, t: &Type) -> bool {
        let registration = self.reading == Reading::Registration;
        let (p, t) = if registration {
            (p, t)
        } else {
            (strip_parens(p), strip_parens(t))
        };
        if let Some(var) = self.vars.type_var(p) {
            return self.bind(var, Value::Type(t.clone()));
        }
        match (p, t) {
            (Type::Path(pp), Type::Path(tp)) if pp.qself.is_none() && tp.qself.is_none() => {
                self.path(&pp.path, &tp.path)
            }
            (Type::Reference(pr), Type::Reference(tr)) => {
                pr.mutability.is_some() == tr.mutability.is_some()
                    && (registration
                        || self.opt_lifetime(pr.lifetime.as_ref(), tr.lifetime.as_ref()))
                    && self.ty(&pr.elem, &tr.elem)
            }
            (Type::Tuple(pt), Type::Tuple(tt)) => {
                pt.elems.len() == tt.elems.len()
                    && pt.elems.iter().zip(&tt.elems).all(|(a, b)| self.ty(a, b))
            }
            _ if registration => match (p, t) {
                (Type::Paren(pp), _) => self.ty(&pp.elem, t),
                (_, Type::Paren(tp)) => self.ty(p, &tp.elem),
                (
                    Type::Path(TypePath {
                        qself: Some(pq),
                        path: pp,
                    }),
                    Type::Path(TypePath {
                        qself: Some(tq),
                        path: tp,
                    }),
                ) => pq.position == tq.position && self.ty(&pq.ty, &tq.ty) && self.segments(pp, tp),
                _ => spelling(p) == spelling(t),
            },
            (Type::Ptr(pp), Type::Ptr(tp)) => {
                pp.mutability.is_some() == tp.mutability.is_some() && self.ty(&pp.elem, &tp.elem)
            }
            (Type::Slice(ps), Type::Slice(ts)) => self.ty(&ps.elem, &ts.elem),
            (Type::Array(pa), Type::Array(ta)) => {
                self.ty(&pa.elem, &ta.elem) && self.expr(&pa.len, &ta.len)
            }
            _ => self.opaque(
                &Names::of_type(p),
                &Names::of_type(t),
                spelling(p) == spelling(t),
            ),
        }
    }

    fn path(&mut self, p: &Path, t: &Path) -> bool {
        if self.reading == Reading::Registration {
            return self.segments(p, t);
        }
        // `self::Foo` is `Foo`.
        let (mut p, mut t) = (p.clone(), t.clone());
        crate::names::strip_leading_self(&mut p);
        crate::names::strip_leading_self(&mut t);
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
        self.segments(&p, &t)
    }

    /// Two paths segment for segment: the same idents, and arguments that match.
    fn segments(&mut self, p: &Path, t: &Path) -> bool {
        p.leading_colon.is_some() == t.leading_colon.is_some()
            && p.segments.len() == t.segments.len()
            && p.segments
                .iter()
                .zip(&t.segments)
                .all(|(ps, ts)| ps.ident == ts.ident && self.args(&ps.arguments, &ts.arguments))
    }

    pub(crate) fn args(&mut self, p: &PathArguments, t: &PathArguments) -> bool {
        match (p, t) {
            (PathArguments::None, PathArguments::None) => true,
            (PathArguments::AngleBracketed(pa), PathArguments::AngleBracketed(ta)) => {
                if self.reading == Reading::Registration {
                    let (pa, ta): (Vec<_>, Vec<_>) =
                        (pa.args.iter().collect(), ta.args.iter().collect());
                    self.positional(&pa, &ta)
                } else {
                    self.arg_list(&pa.args, &ta.args)
                }
            }
            _ if self.reading == Reading::Registration => false,
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

    /// Arguments as the registration check reads them: argument for argument, a lifetime against
    /// any lifetime, a const argument by spelling.
    fn positional(&mut self, p: &[&GenericArgument], t: &[&GenericArgument]) -> bool {
        p.len() == t.len()
            && p.iter().zip(t).all(|(a, b)| match (a, b) {
                (GenericArgument::Type(a), GenericArgument::Type(b)) => self.ty(a, b),
                (GenericArgument::Lifetime(_), GenericArgument::Lifetime(_)) => true,
                (GenericArgument::Const(a), GenericArgument::Const(b)) => {
                    spelling(a) == spelling(b)
                }
                _ => false,
            })
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

// ----- substituting -----------------------------------------------------------------------------

/// Replace the reached impl's parameters (`vars`) by what `subst` maps them to; `failed` when one
/// cannot be said ([`Reading::Premises`] only).
pub(crate) struct Apply<'a> {
    subst: &'a Subst,
    vars: &'a Params,
    reading: Reading,
    pub(crate) failed: bool,
}

impl<'a> Apply<'a> {
    pub(crate) fn new(subst: &'a Subst, vars: &'a Params, reading: Reading) -> Self {
        Apply {
            subst,
            vars,
            reading,
            failed: false,
        }
    }

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

/// `ty`, parenthesized if it could not otherwise stand where a type parameter stood (`&'a T` with
/// `T` = `dyn A + B`).
pub(crate) fn parenthesize(ty: Type) -> Type {
    match ty {
        Type::TraitObject(_) | Type::ImplTrait(_) => Type::Paren(syn::TypeParen {
            paren_token: Default::default(),
            elem: Box::new(ty),
        }),
        other => other,
    }
}

impl VisitMut for Apply<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if self.reading == Reading::Registration {
            // Only a parameter that is the whole type, as written: `T::Out` stays as it is.
            if let Type::Path(TypePath { qself: None, path }) = ty {
                if let Some(Value::Type(t)) = path
                    .get_ident()
                    .and_then(|i| self.subst.get(&i.to_string()))
                {
                    *ty = t.clone();
                    return;
                }
            }
            return syn::visit_mut::visit_type_mut(self, ty);
        }
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

    // The registration reading's `vars` are empty, so the three below leave its match alone.

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
