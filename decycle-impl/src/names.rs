//! How a `#[decycle]` module names its own items, as both engines (and [`crate::analysis`]) read
//! it.
//!
//! A cycle participant — a routed trait, or a type implementing one — is named by its **bare** name
//! or by the no-op `self::` form of it. Any other path (`crate::m::Term`, `super::Term`,
//! `::dep::Term`) names the item itself, which merely shares the last segment: that is the opt-out,
//! and such a path takes no part in the cycle.

use std::collections::HashSet;
use syn::visit::Visit;
use syn::{
    GenericParam, Generics, Ident, Path, PathArguments, TraitBound, Type, TypeParamBound, TypePath,
    UseGlob, UseTree,
};

/// Strips a leading, argument-less `self` segment (`self::Trait` -> `Trait`,
/// `self::Trait::method` -> `Trait::method`) so a `self::`-qualified reference to a
/// #[decycle] trait is recognized the same way the bare name is. A leading `self` is only
/// ever a no-op module-path prefix (it can't itself carry generic arguments), so stripping
/// it is always semantics-preserving.
pub(crate) fn strip_leading_self(path: &mut Path) {
    if path.leading_colon.is_none()
        && path.segments.len() > 1
        && path.segments[0].ident == "self"
        && matches!(path.segments[0].arguments, PathArguments::None)
    {
        path.segments = path.segments.iter().skip(1).cloned().collect();
    }
}

/// The one ident `path` spells once a no-op leading `self::` is stripped ([`strip_leading_self`]):
/// `Tr` and `self::Tr` spell `Tr`; `crate::m::Tr` and `super::Tr` spell none.
///
/// This is the spelling the ranked engine (and [`crate::analysis`]) take a routed trait by, in an
/// impl header and in a bound. A leading `::` is not looked at, so `::Tr` spells `Tr` here;
/// [`bare_ident`] is the same rule with a leading `::` refused.
pub(crate) fn routed_ident(path: &Path) -> Option<&Ident> {
    let mut segments = path.segments.iter();
    let first = segments.next()?;
    match (segments.next(), segments.next()) {
        (None, _) => Some(&first.ident),
        (Some(second), None)
            if path.leading_colon.is_none()
                && first.ident == "self"
                && matches!(first.arguments, PathArguments::None) =>
        {
            Some(&second.ident)
        }
        _ => None,
    }
}

/// Is `bound` a trait bound whose path spells ([`routed_ident`]) one of `routed`? A crate-rooted
/// or `super::` spelling is the caller's documented opt-out and never a cycle edge.
pub(crate) fn is_routed_bound(bound: &TypeParamBound, routed: &HashSet<Ident>) -> bool {
    let TypeParamBound::Trait(TraitBound { path, .. }) = bound else {
        return false;
    };
    routed_ident(path).is_some_and(|ident| routed.contains(ident))
}

/// The ident `path` names a cycle participant by: its bare name, or the no-op `self::` form. Any
/// other path (`crate::m::Tr`, `super::Term`, `::other::B`) names the item itself, the opt-out.
pub(crate) fn bare_ident(path: &Path) -> Option<&Ident> {
    if path.leading_colon.is_some() {
        return None;
    }
    routed_ident(path)
}

/// The head ident of a type, but **only** when the type is spelled as a module-local item — a bare
/// single-segment path, or its no-op `self::`-qualified form ([`bare_ident`]). `None` for a
/// non-path type (`&Stmt`, `(A, B)`) or a `<T as Tr>::X` qself.
///
/// For the places that compare a head against a set of the *processed module's own* idents. Using
/// the last segment there confuses `crate::other::B` with the local `B`: a foreign type would join
/// a sibling's premise group (`sharing_sources`), or pass the "this bound is rank-lowerable" check
/// (`validate_impl_where_bounds`, `peel`) and get rank-lowered into a `TrRanked<…>` wall.
pub(crate) fn local_type_head_ident(ty: &Type) -> Option<Ident> {
    match ty {
        Type::Path(TypePath { qself: None, path }) => bare_ident(path).cloned(),
        _ => None,
    }
}

/// Does `path` name one of `names` as a **module-local** item ([`bare_ident`])?
///
/// Only those two spellings can denote an item of the module being processed. A path rooted
/// anywhere else — `crate::other::Stmt`, `super::Stmt`, `::dep::Stmt` — reaches a *different* item
/// that merely happens to share its last segment, and must be treated as an ordinary outer type.
pub(crate) fn path_names_local_ident(path: &Path, names: &HashSet<Ident>) -> bool {
    bare_ident(path).is_some_and(|ident| names.contains(ident))
}

/// Collects every path naming a participant (`is_participant`) by its bare name ([`bare_ident`]),
/// outermost first, wherever it is visited from.
///
/// A path that names none still has its arguments visited, and so does one that names one: a
/// member nested inside a foreign wrapper (`crate::other::Wrap<Stmt>`) or inside another member's
/// arguments (`Stmt<Expr>`) is found.
pub(crate) struct BareNamed<'p, 'ast> {
    is_participant: &'p dyn Fn(&Ident) -> bool,
    pub(crate) found: Vec<&'ast TypePath>,
}

impl<'p> BareNamed<'p, '_> {
    pub(crate) fn new(is_participant: &'p dyn Fn(&Ident) -> bool) -> Self {
        BareNamed {
            is_participant,
            found: Vec::new(),
        }
    }
}

impl<'ast> Visit<'ast> for BareNamed<'_, 'ast> {
    fn visit_type_path(&mut self, tp: &'ast TypePath) {
        if tp.qself.is_none() && bare_ident(&tp.path).is_some_and(|i| (self.is_participant)(i)) {
            self.found.push(tp);
        }
        syn::visit::visit_type_path(self, tp);
    }
}

/// [`BareNamed`] over `ty`: the paths inside it naming a participant, outermost first.
pub(crate) fn bare_named_in<'ast>(
    ty: &'ast Type,
    is_participant: &dyn Fn(&Ident) -> bool,
) -> Vec<&'ast TypePath> {
    let mut v = BareNamed::new(is_participant);
    v.visit_type(ty);
    v.found
}

/// The idents of `generics`' type parameters — a bound on one targets the parameter, never a
/// participant it may shadow.
pub(crate) fn type_param_idents(generics: &Generics) -> HashSet<Ident> {
    generics
        .params
        .iter()
        .filter_map(|p| match p {
            GenericParam::Type(t) => Some(t.ident.clone()),
            _ => None,
        })
        .collect()
}

/// See through `(T)` and the invisible groups a macro-expanded type can be wrapped in.
pub(crate) fn strip_parens(ty: &Type) -> &Type {
    match ty {
        Type::Paren(p) => strip_parens(&p.elem),
        Type::Group(g) => strip_parens(&g.elem),
        other => other,
    }
}

/// One name a `use` tree binds: `use a::T;` binds `T`, `use a::T as R;` binds `R` (for `T`), and a
/// glob `use a::*;` binds no name of its own.
pub(crate) enum UseLeaf<'a> {
    Name(&'a Ident),
    Rename {
        original: &'a Ident,
        local: &'a Ident,
    },
    Glob(&'a UseGlob),
}

impl<'a> UseLeaf<'a> {
    /// The name the leaf binds in the module; none for a glob.
    pub(crate) fn local(&self) -> Option<&'a Ident> {
        match self {
            UseLeaf::Name(ident) | UseLeaf::Rename { local: ident, .. } => Some(ident),
            UseLeaf::Glob(_) => None,
        }
    }
}

/// The leaves of `tree`, in source order: what a `#[decycle] use` routes, read the same way by both
/// engines, [`crate::analysis`] and `crate::growth`.
pub(crate) fn use_leaves(tree: &UseTree) -> Vec<UseLeaf<'_>> {
    fn walk<'a>(tree: &'a UseTree, out: &mut Vec<UseLeaf<'a>>) {
        match tree {
            UseTree::Path(p) => walk(&p.tree, out),
            UseTree::Name(n) => out.push(UseLeaf::Name(&n.ident)),
            UseTree::Rename(r) => out.push(UseLeaf::Rename {
                original: &r.ident,
                local: &r.rename,
            }),
            UseTree::Glob(g) => out.push(UseLeaf::Glob(g)),
            UseTree::Group(g) => g.items.iter().for_each(|t| walk(t, out)),
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

/// The names `tree` binds, in source order: [`UseLeaf::local`] of each leaf (a glob binds none).
pub(crate) fn use_names(tree: &UseTree) -> impl Iterator<Item = &Ident> {
    use_leaves(tree).into_iter().filter_map(|leaf| leaf.local())
}
