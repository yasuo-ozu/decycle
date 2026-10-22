//! Peel container wrappers off a cyclic where-bound so the ranked engine has a head it can descend.
//!
//! The ranked engine rewrites a cyclic bound `X: Tr` to `X: TrRanked<Rank>`, which resolves only when
//! `X`'s **head** type has a ranked impl in this module. A bound written on a wrapped type —
//! `Vec<Box<Stmt<S>>>: Tr`, the natural shape when the obligation is generated from a struct field —
//! has head `Vec`, which has no ranked impl, so the rewrite produces nothing rustc can solve.
//!
//! The information needed to fix it is already present, though: the cycle member is right there
//! inside the type arguments. `Vec<Box<Stmt<S>>>: Tr` and `Stmt<S>: Tr` are the same obligation as far
//! as the cycle is concerned — the container's own impl supplies the rest — so this pass rewrites the
//! former into the latter. Only then is `validate_impl_where_bounds` able to insist that every
//! surviving cyclic bound has a rankable head, because by that point the ones that *could* be made
//! rankable already are.
//!
//! The contract a peel relies on is that **a wrapper implements the trait when its type arguments
//! do**. So a wrapper is replaced by the obligations of its parts, and nothing is dropped: a part
//! that is a cycle member becomes an edge, and a part that contains no member at all stays an
//! ordinary premise on the original trait, as written — `(B, crate::m::C): Tr` peels to `B: Tr` and
//! `crate::m::C: Tr`, and `(B, u8): Tr` to `B: Tr` and `u8: Tr`. Dropping those parts (as this pass
//! once did) left a call the body still makes with no obligation behind it, which the ranked engine
//! could only discover at run time.
//!
//! Bounds that are not cyclic, and cyclic bounds that already name a rankable head (or target `Self`
//! or one of the impl's own type parameters), are left as written — but for the spelling of a cycle
//! member's head, which is made bare (`self::Term` → `Term`): the impls are re-emitted two modules
//! deeper, where only the bare spelling still names the member.

use crate::names::BareNamed;
use std::collections::HashSet;
use syn::punctuated::Punctuated;
use syn::visit::Visit;
use syn::{
    GenericArgument, Ident, ItemImpl, PathArguments, Token, TraitBound, Type, TypeParamBound,
    TypePath, WherePredicate,
};
use template_quote::quote;

/// Rewrite every peelable cyclic bound of `item_impl` in place.
pub(crate) fn peel_cyclic_bounds(
    item_impl: &mut ItemImpl,
    all_traits: &HashSet<Ident>,
    cycle_self_heads: &HashSet<Ident>,
    is_local_target: &dyn Fn(&Type) -> bool,
) {
    let Some(where_clause) = &mut item_impl.generics.where_clause else {
        return;
    };
    let mut out: Punctuated<WherePredicate, Token![,]> = Punctuated::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |p: WherePredicate, out: &mut Punctuated<WherePredicate, Token![,]>| {
        if seen.insert(quote!(#p).to_string()) {
            out.push(p);
        }
    };

    for pred in where_clause.predicates.iter() {
        let WherePredicate::Type(pt) = pred else {
            push(pred.clone(), &mut out);
            continue;
        };
        // Already fine: `Self` / an impl type-param (the engine handles those directly).
        if is_local_target(&pt.bounded_ty) {
            push(pred.clone(), &mut out);
            continue;
        }
        // Already fine too: a head that is itself a cycle member — recognised by the BARE (or
        // `self::`-qualified) spelling only, exactly like `cycle_types_within` below. Keying on the
        // last segment made `crate::other::Stmt` pass for the local member `Stmt`.
        if crate::names::local_type_head_ident(&pt.bounded_ty)
            .is_some_and(|h| cycle_self_heads.contains(&h))
        {
            // `self::Term` and `Term` are the same member; emit the bare spelling, as the peeled
            // targets below are. A `self::` path is depth-fragile, so `nesting` would otherwise
            // lift it to an alias, and `finalize` would no longer see the member's head. (Unless
            // the bare name is the impl's own type parameter, which then shadows the member.)
            let mut bare = pt.bounded_ty.clone();
            if let Type::Path(TypePath { qself: None, path }) = &mut bare {
                crate::names::strip_leading_self(path);
            }
            let mut pred = pred.clone();
            if let WherePredicate::Type(pt) = &mut pred {
                if !is_local_target(&bare) {
                    pt.bounded_ty = bare;
                }
            }
            push(pred, &mut out);
            continue;
        }
        let (mut cyclic, other): (Vec<TypeParamBound>, Vec<TypeParamBound>) = pt
            .bounds
            .iter()
            .cloned()
            .partition(|b| crate::names::is_routed_bound(b, all_traits));
        // `self::Tr` and `Tr` mean the same edge; emit the canonical bare spelling.
        for b in cyclic.iter_mut() {
            if let TypeParamBound::Trait(TraitBound { path, .. }) = b {
                crate::names::strip_leading_self(path);
            }
        }
        if cyclic.is_empty() {
            push(pred.clone(), &mut out);
            continue;
        }
        let mut parts = Parts::default();
        parts.decompose(&pt.bounded_ty, cycle_self_heads);
        if parts.members.is_empty() {
            // Nothing to peel to. Left as written so `validate_impl_where_bounds` can report it
            // against the bound the caller actually wrote.
            push(pred.clone(), &mut out);
            continue;
        }
        // The predicate's own `for<'a, …>` binder comes along: a peeled target can mention a bound
        // lifetime (`for<'a> Wrapper<Sub<'a>>: Tr` ⇒ `for<'a> Sub<'a>: Tr`), which would otherwise be
        // undeclared (E0261).
        let binder = &pt.lifetimes;
        // A member is an edge; a member-free part is a premise on the original trait, spelled as
        // written. Both carry the trait bare: a bare bound on a type that is no cycle member is
        // exactly how a premise reaches `finalize` (it keeps it, un-lowered).
        for target in parts.members.iter().chain(&parts.premises) {
            push(syn::parse_quote!(#binder #target: #(#cyclic)+*), &mut out);
        }
        // An unrelated extra bound (`T: Tr + Clone`) stays on the ORIGINAL type — peeling it too
        // would silently move a constraint onto a different type.
        if !other.is_empty() {
            let bounded = &pt.bounded_ty;
            push(syn::parse_quote!(#binder #bounded: #(#other)+*), &mut out);
        }
    }
    where_clause.predicates = out;
}

/// The obligations a wrapped bound stands for, by the peel contract: a wrapper implements the trait
/// when its type arguments do.
#[derive(Default)]
struct Parts {
    /// The cycle members reached, in their bare spelling, outermost first: the edges.
    members: Vec<Type>,
    /// The parts that contain no member, as written: premises on the original trait.
    premises: Vec<Type>,
    seen: HashSet<String>,
}

impl Parts {
    fn member(&mut self, ty: Type) {
        if self.seen.insert(quote!(#ty).to_string()) {
            self.members.push(ty);
        }
    }

    fn premise(&mut self, ty: &Type) {
        if self.seen.insert(quote!(#ty).to_string()) {
            self.premises.push(ty.clone());
        }
    }

    /// Split `ty` into the obligations of its parts.
    ///
    /// - A cycle member (bare or `self::`) is an edge. Members inside its own arguments are edges
    ///   too, as they always were (`Vec<Stmt<Expr>>: Tr` ⇒ `Stmt<Expr>: Tr`, `Expr: Tr`), but the
    ///   rest of its arguments are the member's business, not parts of a wrapper.
    /// - A type that contains no member is a premise, as written: `crate::m::C`, `u8`.
    /// - A wrapper that contains a member is replaced by the obligations of its parts: the type
    ///   arguments of a path, the elements of a tuple, the referent of a reference or pointer, the
    ///   element of an array or slice.
    /// - Anything else that contains a member (`dyn Fn(Stmt)`, `<Stmt as Tr>::Out`, a member inside
    ///   an associated-type binding) is not a wrapper this contract covers: its members are edges,
    ///   as before, and nothing else is read from it.
    fn decompose(&mut self, ty: &Type, heads: &HashSet<Ident>) {
        let within = cycle_types_within(ty, heads);
        if within.is_empty() {
            self.premise(ty);
            return;
        }
        let is_member = matches!(ty, Type::Path(TypePath { qself: None, path })
            if crate::names::path_names_local_ident(path, heads));
        if is_member {
            within.into_iter().for_each(|t| self.member(t));
            return;
        }
        match ty {
            Type::Path(TypePath { qself: None, path }) => {
                for seg in &path.segments {
                    match &seg.arguments {
                        PathArguments::AngleBracketed(args) => {
                            for arg in &args.args {
                                match arg {
                                    GenericArgument::Type(t) => self.decompose(t, heads),
                                    // A lifetime or a const names no type; a binding or a
                                    // constraint is no part of the wrapper.
                                    other => {
                                        self.members_in(heads, |v| v.visit_generic_argument(other))
                                    }
                                }
                            }
                        }
                        PathArguments::Parenthesized(args) => self
                            .members_in(heads, |v| v.visit_parenthesized_generic_arguments(args)),
                        PathArguments::None => {}
                    }
                }
            }
            Type::Tuple(t) => t.elems.iter().for_each(|e| self.decompose(e, heads)),
            Type::Reference(r) => self.decompose(&r.elem, heads),
            Type::Ptr(p) => self.decompose(&p.elem, heads),
            Type::Array(a) => self.decompose(&a.elem, heads),
            Type::Slice(s) => self.decompose(&s.elem, heads),
            Type::Paren(p) => self.decompose(&p.elem, heads),
            Type::Group(g) => self.decompose(&g.elem, heads),
            _ => within.into_iter().for_each(|t| self.member(t)),
        }
    }

    /// The members `visit` finds in a node that is no part of a wrapper, as edges.
    fn members_in<'ast>(
        &mut self,
        heads: &HashSet<Ident>,
        visit: impl FnOnce(&mut BareNamed<'_, 'ast>),
    ) {
        let is_member = |i: &Ident| heads.contains(i);
        let mut v = BareNamed::new(&is_member);
        visit(&mut v);
        bare_spelled(v.found).for_each(|t| self.member(t));
    }
}

/// Every cycle-member type appearing anywhere inside `ty`, outermost first (a member found twice is
/// listed twice; [`Parts::member`] keeps the first).
///
/// A member is recognised only by a **bare** (or `self::`-qualified) ident — the same rule
/// [`crate::names::is_routed_bound`] applies to the trait side. A multi-segment path is an outer
/// type in its entirety, even when its last segment matches a member's name: `crate::other::Stmt`
/// is a different item from the local `Stmt`, and peeling it emitted rank-lowering obligations
/// against a foreign type, which surfaced as a per-rank wall of `TrRanked<((((..` errors (or the
/// "not a type the ranked engine can rank-lower" abort) instead of leaving the bound alone.
///
/// Recursion is unaffected: an outer path's generic arguments are still visited, so a genuine
/// member nested inside a foreign wrapper (`crate::other::Wrap<Stmt>`) is still found.
fn cycle_types_within(ty: &Type, cycle_self_heads: &HashSet<Ident>) -> Vec<Type> {
    bare_spelled(crate::names::bare_named_in(ty, &|i| cycle_self_heads.contains(i))).collect()
}

/// The members found, in the canonical BARE spelling, exactly as the trait side is normalised
/// above: a peeled target is re-emitted two modules deeper, where a `self::`-qualified path would
/// resolve against the wrong module.
fn bare_spelled(found: Vec<&TypePath>) -> impl Iterator<Item = Type> + '_ {
    found.into_iter().map(|tp| {
        let mut tp = tp.clone();
        crate::names::strip_leading_self(&mut tp.path);
        Type::Path(tp)
    })
}
