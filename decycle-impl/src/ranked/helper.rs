use proc_macro2::{Span, TokenStream};
use proc_macro_error::*;
use syn::punctuated::Punctuated;
use syn::*;
use template_quote::quote;

/// Rejection for a `#[decycle]` trait named with `Fn(...)`-sugar (`where B: Cb(usize) -> usize`):
/// the ranked-trait rewrite must splice a `Rank` argument, which parenthesized args can't carry.
const PARENTHESIZED_ARGS_MSG: &str = "unsupported parenthesized generic arguments on a #[decycle] trait";

/// Strips a leading, argument-less `self` segment (`self::Trait` -> `Trait`,
/// `self::Trait::method` -> `Trait::method`) so a `self::`-qualified reference to a
/// #[decycle] trait is recognized the same way the bare name is. A leading `self` is only
/// ever a no-op module-path prefix (it can't itself carry generic arguments), so stripping
/// it is always semantics-preserving.
/// The head (outermost path) ident of a type: `Box` for `Box<Stmt>`, `Stmt` for `Stmt<S>`.
/// `None` for a non-path type (`&Stmt`, `(A, B)`) or a `<T as Tr>::X` qself.
///
/// Used to decide whether a bound's *target* is something the ranked chain actually descends
/// through — see `finalize::remove_cyclic_bounds` and `process_module`'s rank-lowerability check.
pub fn type_head_ident(ty: &Type) -> Option<Ident> {
    match ty {
        Type::Path(TypePath { qself: None, path }) => path.segments.last().map(|s| s.ident.clone()),
        _ => None,
    }
}

/// Is `path` **depth-fragile** — i.e. does its meaning change when the item carrying it is re-emitted
/// inside a deeper module? Only `super::`/`self::`-rooted paths are: an absolute (`::a::b`) or
/// `crate::`-rooted path denotes the same item at any depth.
pub fn path_is_depth_fragile(path: &Path) -> bool {
    path.leading_colon.is_none()
        && path
            .segments
            .first()
            .is_some_and(|seg| seg.ident == "super" || seg.ident == "self")
}

pub fn strip_leading_self(path: &mut Path) {
    if path.leading_colon.is_none()
        && path.segments.len() > 1
        && path.segments[0].ident == "self"
        && matches!(path.segments[0].arguments, PathArguments::None)
    {
        path.segments = path.segments.iter().skip(1).cloned().collect();
    }
}

/// Does `path` name one of `names` as a **module-local** item — i.e. is it a bare single-segment
/// path, or its no-op `self::`-qualified form?
///
/// Only those two spellings can denote an item of the module being processed. A path rooted
/// anywhere else — `crate::other::Stmt`, `super::Stmt`, `::dep::Stmt` — reaches a *different* item
/// that merely happens to share its last segment, and must be treated as an ordinary outer type.
///
/// This is the type-side counterpart of `peel::is_bare_cyclic_bound`, which already applies the
/// same bare-or-`self::` rule to the trait side of a bound.
pub fn path_names_local_ident(path: &Path, names: &std::collections::HashSet<Ident>) -> bool {
    if path.leading_colon.is_some() {
        return false;
    }
    let mut probe = path.clone();
    strip_leading_self(&mut probe);
    probe.segments.len() == 1 && names.contains(&probe.segments[0].ident)
}

/// The fresh binding `reduce_pat` mints for a destructured parameter at position `ix`.
///
/// Carries the crate-identity suffix so it cannot collide with a user parameter that happens to be
/// named `__arg_1_`; deterministic across compilations, like every other generated ident here.
fn arg_ident_name(ix: usize) -> String {
    static RANDOM_SUFFIX: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let suffix = RANDOM_SUFFIX.get_or_init(|| crate::name_suffix(crate::get_random()));
    format!("__arg_{ix}_{suffix}")
}

/// Inserts a `Type` as a `GenericArgument::Type` at the given position
/// in the last segment's arguments of `path`.
///
/// `index` is the rank parameter's slot in the trait's *declaration* — i.e. the number of
/// lifetime params, since a declaration must list lifetimes first. The arguments actually written
/// at a use site need not match that: path lifetime arguments are elided all-or-nothing (a partial
/// list is E0107), and in expression position (`Tr::f(..)`, `<B as Tr>::f(..)`) they almost always
/// are. So the correct slot is `index` clamped to the number of leading lifetime arguments PRESENT
/// — 0 when they are elided.
///
/// Without the clamp, a trait with one lifetime param produced `args.insert(1, ..)` on an empty
/// list, which panics (`Punctuated::insert: index out of range`) — a bare proc-macro panic, with no
/// span, on legal code. When some args were present it was worse than a panic: `Tr::<u32>::f` on
/// `trait Tr<'a, T>` inserted at 1 and silently emitted `TrRanked<u32, Rank>`, putting the user's
/// type in the rank slot. Clamping also keeps the argument ahead of any associated-type binding,
/// which must follow all generic arguments.
pub fn path_insert_type_arg(path: &mut Path, index: usize, ty: Type) {
    let last_seg = path.segments.last_mut().unwrap();
    let arg = GenericArgument::Type(ty);
    match &mut last_seg.arguments {
        PathArguments::None => {
            let mut args = Punctuated::new();
            // Nothing was written, so every lifetime is elided: the rank argument goes first.
            args.insert(0, arg);
            last_seg.arguments = PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                colon2_token: None,
                lt_token: Default::default(),
                args,
                gt_token: Default::default(),
            });
        }
        PathArguments::AngleBracketed(ref mut angle_args) => {
            // Clamp to the lifetime arguments actually spelled here (see the doc comment): the
            // rank slot is declaration-relative, the written list may have elided its lifetimes.
            let leading_lifetimes = angle_args
                .args
                .iter()
                .take_while(|a| matches!(a, GenericArgument::Lifetime(_)))
                .count();
            angle_args.args.insert(index.min(leading_lifetimes), arg);
        }
        // A #[decycle] trait referenced with `Fn(...)`-sugar (`where B: Cb(usize) -> usize`)
        // reaches here through `TraitReplacer` (the where-clause/body rewriter): it steals
        // the ORIGINAL `Parenthesized` arguments onto the ranked-trait replacement path
        // before this call is meant to insert the Rank argument. Silently doing nothing
        // (the old behavior) left the Rank argument out entirely, producing a ranked-trait
        // reference desugared as `CbRanked<(usize,), Output = usize>` — missing its Rank
        // parameter — which cascades into confusing, seemingly unrelated errors downstream
        // (E0658/E0220/E0277) instead of naming the actual problem. Abort here instead,
        // matching `PathArgumentsScheme::insert`'s identical rejection for an impl's own
        // (syntactically distinct, but equally unsupported) parenthesized trait reference.
        PathArguments::Parenthesized(pa) => {
            abort!(pa, PARENTHESIZED_ARGS_MSG)
        }
    }
}

pub trait FnArgScheme {
    fn reduce_pat(&mut self, ix: usize);
    fn variable(&self) -> TokenStream;
}

impl FnArgScheme for FnArg {
    fn reduce_pat(&mut self, ix: usize) {
        if let FnArg::Typed(PatType { pat, .. }) = self {
            match pat.as_mut() {
                // Keep the ident (and its `mut`, which is signature-only and doesn't
                // affect the caller), but drop `by_ref`/subpatterns — those bind
                // additional names that only make sense in the original body, not at
                // the call-argument position `variable()` quotes this pat into.
                Pat::Ident(pat_ident) => {
                    pat_ident.by_ref = None;
                    pat_ident.subpat = None;
                }
                _ => {
                    // Nonce-suffixed: `__arg_{ix}_` alone is call-site hygiene, so a user
                    // parameter literally named that collided with it (E0415, "bound more than
                    // once"). Same mechanism the rest of the crate's generated idents use — a
                    // pure function of the crate identity, so it stays deterministic across
                    // compilations (reproducible builds, stable trybuild snapshots).
                    **pat = Pat::Ident(PatIdent {
                        ident: Ident::new(&arg_ident_name(ix), Span::call_site()),
                        attrs: vec![],
                        by_ref: None,
                        mutability: None,
                        subpat: None,
                    });
                }
            }
        }
    }

    fn variable(&self) -> TokenStream {
        match self {
            FnArg::Typed(PatType { pat, .. }) => {
                let Pat::Ident(pat_ident) = pat.as_ref() else {
                    unreachable!("reduce_pat always leaves a bare Pat::Ident");
                };
                // Emit only the ident: `mut`/`by_ref`/subpatterns are signature
                // decorations, not valid in a call-argument expression position.
                let ident = &pat_ident.ident;
                quote!(#ident)
            }
            FnArg::Receiver(Receiver { self_token, .. }) => {
                quote!(#self_token)
            }
        }
    }
}

pub trait PathArgumentsScheme {
    fn insert(&self, ix: usize, ty: Type) -> PathArguments;
}

impl PathArgumentsScheme for PathArguments {
    fn insert(&self, index: usize, ty: Type) -> PathArguments {
        match self {
            PathArguments::None => {
                assert_eq!(index, 0);
                PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                    colon2_token: None,
                    lt_token: Default::default(),
                    args: core::iter::once(GenericArgument::Type(ty)).collect(),
                    gt_token: Default::default(),
                })
            }
            PathArguments::AngleBracketed(angle_args) => {
                let mut angle_args = angle_args.clone();
                angle_args.args.insert(index, GenericArgument::Type(ty));
                PathArguments::AngleBracketed(angle_args)
            }
            // Only reachable when a #[decycle] trait is named with `Fn(...)`-sugar syntax
            // (`impl FnLike(A) -> B for X`), which isn't a supported form of a decycle
            // trait bound — a clean compile error instead of an internal panic.
            PathArguments::Parenthesized(pa) => {
                abort!(pa, PARENTHESIZED_ARGS_MSG)
            }
        }
    }
}

pub trait GenericsScheme {
    fn push_predicate(&self, predicate: WherePredicate) -> Self;
    fn insert(&self, index: usize, param: TypeParam) -> Self;
    fn impl_generics(&self) -> TokenStream;
    fn ty_generics(&self) -> PathArguments;
}

impl GenericsScheme for Generics {
    fn push_predicate(&self, predicate: WherePredicate) -> Self {
        let mut g = self.clone();
        g.where_clause
            .get_or_insert(WhereClause {
                where_token: Default::default(),
                predicates: Default::default(),
            })
            .predicates
            .push(predicate);
        g
    }

    fn insert(&self, index: usize, param: TypeParam) -> Self {
        let mut generics = self.clone();
        generics.params.insert(index, GenericParam::Type(param));
        if generics.lt_token.is_none() && !generics.params.is_empty() {
            generics.lt_token = Some(Default::default());
            generics.gt_token = Some(Default::default());
        }
        generics
    }

    fn impl_generics(&self) -> TokenStream {
        let (impl_generics, _, _) = self.split_for_impl();
        quote!(#impl_generics)
    }

    fn ty_generics(&self) -> PathArguments {
        if self.lt_token.is_none() {
            PathArguments::None
        } else {
            let args: Punctuated<GenericArgument, Token![,]> = self
                .params
                .iter()
                .map(|param| match param {
                    GenericParam::Lifetime(lt) => GenericArgument::Lifetime(lt.lifetime.clone()),
                    GenericParam::Type(tp) => {
                        let ident = &tp.ident;
                        GenericArgument::Type(syn::parse_quote!(#ident))
                    }
                    GenericParam::Const(cp) => {
                        let ident = &cp.ident;
                        GenericArgument::Const(syn::parse_quote!(#ident))
                    }
                })
                .collect();
            PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                colon2_token: None,
                lt_token: self.lt_token.unwrap_or_default(),
                args,
                gt_token: self.gt_token.unwrap_or_default(),
            })
        }
    }
}

impl GenericsScheme for Path {
    fn push_predicate(&self, _predicate: WherePredicate) -> Self {
        unimplemented!()
    }

    fn insert(&self, index: usize, param: TypeParam) -> Self {
        let mut path = self.clone();
        let ident = &param.ident;
        let ty = Type::Path(TypePath {
            qself: None,
            path: syn::parse_quote!(#ident),
        });
        path_insert_type_arg(&mut path, index, ty);
        path
    }

    fn impl_generics(&self) -> TokenStream {
        quote!()
    }

    fn ty_generics(&self) -> PathArguments {
        if let Some(last_segment) = self.segments.last() {
            last_segment.arguments.clone()
        } else {
            PathArguments::None
        }
    }
}
