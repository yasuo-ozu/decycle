use proc_macro2::{Span, TokenStream};
use proc_macro_error::*;
use syn::punctuated::Punctuated;
use syn::*;
use template_quote::quote;

/// Strips a leading, argument-less `self` segment (`self::Trait` -> `Trait`,
/// `self::Trait::method` -> `Trait::method`) so a `self::`-qualified reference to a
/// #[decycle] trait is recognized the same way the bare name is. A leading `self` is only
/// ever a no-op module-path prefix (it can't itself carry generic arguments), so stripping
/// it is always semantics-preserving.
pub fn strip_leading_self(path: &mut Path) {
    if path.leading_colon.is_none()
        && path.segments.len() > 1
        && path.segments[0].ident == "self"
        && matches!(path.segments[0].arguments, PathArguments::None)
    {
        path.segments = path.segments.iter().skip(1).cloned().collect();
    }
}

/// Inserts a `Type` as a `GenericArgument::Type` at the given position
/// in the last segment's arguments of `path`.
pub fn path_insert_type_arg(path: &mut Path, index: usize, ty: Type) {
    let last_seg = path.segments.last_mut().unwrap();
    let arg = GenericArgument::Type(ty);
    match &mut last_seg.arguments {
        PathArguments::None => {
            let mut args = Punctuated::new();
            args.insert(index, arg);
            last_seg.arguments = PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                colon2_token: None,
                lt_token: Default::default(),
                args,
                gt_token: Default::default(),
            });
        }
        PathArguments::AngleBracketed(ref mut angle_args) => {
            angle_args.args.insert(index, arg);
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
            abort!(pa, "unsupported parenthesized generic arguments on a #[decycle] trait")
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
                    **pat = Pat::Ident(PatIdent {
                        ident: Ident::new(&format!("__arg_{ix}_"), Span::call_site()),
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
                abort!(pa, "unsupported parenthesized generic arguments on a #[decycle] trait")
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
