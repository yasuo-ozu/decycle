use proc_macro2::{Span, TokenStream};
use syn::punctuated::Punctuated;
use syn::*;
use template_quote::quote;

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
        PathArguments::Parenthesized(_) => {}
    }
}

pub trait TraitItemScheme {
    fn remove_default_body(&self) -> Self;
}

impl TraitItemScheme for TraitItem {
    fn remove_default_body(&self) -> Self {
        let mut ret = self.clone();
        if let TraitItem::Fn(TraitItemFn {
            default,
            semi_token,
            ..
        }) = &mut ret
        {
            *default = None;
            *semi_token = Some(Default::default());
        }
        ret
    }
}

pub trait FnArgScheme {
    fn reduce_pat(&mut self, ix: usize);
    fn variable(&self) -> TokenStream;
}

impl FnArgScheme for FnArg {
    fn reduce_pat(&mut self, ix: usize) {
        if let FnArg::Typed(PatType { pat, .. }) = self {
            if !matches!(pat.as_ref(), Pat::Ident(_)) {
                *pat = Box::new(Pat::Ident(PatIdent {
                    ident: Ident::new(&format!("__arg_{ix}_"), Span::call_site()),
                    attrs: vec![],
                    by_ref: None,
                    mutability: None,
                    subpat: None,
                }));
            }
        }
    }

    fn variable(&self) -> TokenStream {
        match self {
            FnArg::Typed(PatType { pat, .. }) => {
                assert!(matches!(pat.as_ref(), Pat::Ident(_)));
                quote!(#pat)
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
            PathArguments::Parenthesized(_) => panic!(),
        }
    }
}

pub trait GenericsScheme {
    fn push_predicate(&self, predicate: WherePredicate) -> Self;
    fn insert(&self, index: usize, param: TypeParam) -> Self;
    fn insert_last(&self, param: TypeParam) -> Self;
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

    fn insert_last(&self, param: TypeParam) -> Self {
        let ix = self
            .params
            .iter()
            .take_while(|p| {
                matches!(
                    p,
                    GenericParam::Lifetime(_)
                        | GenericParam::Type(TypeParam { eq_token: None, .. })
                        | GenericParam::Const(ConstParam { eq_token: None, .. })
                )
            })
            .count();
        self.insert(ix, param)
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

    fn insert_last(&self, _param: TypeParam) -> Self {
        unimplemented!()
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
