//! Rendering `syn::Generics` (or a single `GenericParam`) to token form. Both engines emit
//! generated impls / aliases / markers / traits whose parameter lists must mirror a source item's
//! generics, and each engine independently grew the same small family of "stringify a param list"
//! helpers. This module is the single home for them.
//!
//! The only axis of real variation is how a *type* parameter's bounds are rendered in a
//! DECLARATION position — captured by [`DeclBounds`]. Lifetime and const params render the same in
//! every mode (`'a` / `const N: T`, defaults always stripped), except that [`DeclBounds::Keep`]
//! additionally preserves lifetime *bounds* (`'a: 'b`) while the bound-dropping modes reduce a
//! lifetime param to the bare lifetime.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{GenericParam, Generics};

/// How a *type* parameter's bounds are rendered when the param appears in a declaration.
#[derive(Clone, Copy)]
pub(crate) enum DeclBounds {
    /// Drop all bounds: `T`. Also drops lifetime bounds. (structural terminator / impl generics)
    Bare,
    /// No user bounds, but relax the implicit `Sized`: `T: ?Sized`. Also drops lifetime bounds.
    /// (ranked marker / alias generics — the param is only held in `PhantomData<*const T>` or used
    /// behind a reference, both `?Sized`-tolerant, so a `V: ?Sized` cycle must not fail E0277 at
    /// the declaration merely from being named.)
    Unsized,
    /// Keep bounds, strip defaults: `T: Bound`, `'a: 'b`. (ranked re-entry fn / ranked-trait decl)
    Keep,
}

/// One generic param as a declaration, per `bounds` mode. Const params always render
/// `const N: T` (default stripped) regardless of mode.
pub(crate) fn param_decl(p: &GenericParam, bounds: DeclBounds) -> TokenStream {
    match p {
        GenericParam::Lifetime(l) => match bounds {
            DeclBounds::Keep => quote!(#l), // keep lifetime bounds (`'a: 'b`)
            DeclBounds::Bare | DeclBounds::Unsized => {
                let lt = &l.lifetime;
                quote!(#lt)
            }
        },
        GenericParam::Const(c) => {
            let mut c = c.clone();
            c.default = None;
            c.eq_token = None;
            quote!(#c)
        }
        GenericParam::Type(t) => match bounds {
            DeclBounds::Bare => {
                let id = &t.ident;
                quote!(#id)
            }
            DeclBounds::Unsized => {
                let id = &t.ident;
                quote!(#id: ?::core::marker::Sized)
            }
            DeclBounds::Keep => {
                let mut t = t.clone();
                t.default = None;
                t.eq_token = None;
                quote!(#t)
            }
        },
    }
}

/// Every param of `g` as a comma-separated declaration list (no angle brackets): e.g. for
/// [`DeclBounds::Bare`] → `'a, Span, const N: usize`.
pub(crate) fn params_decl(g: &Generics, bounds: DeclBounds) -> TokenStream {
    let parts = g.params.iter().map(|p| param_decl(p, bounds));
    quote!( #(#parts),* )
}

/// Every param of `g` as a comma-separated USE list (no angle brackets): `'a, Span, N`.
pub(crate) fn params_use(g: &Generics) -> TokenStream {
    let parts = g.params.iter().map(|p| match p {
        GenericParam::Lifetime(l) => {
            let lt = &l.lifetime;
            quote!(#lt)
        }
        GenericParam::Type(t) => {
            let id = &t.ident;
            quote!(#id)
        }
        GenericParam::Const(c) => {
            let id = &c.ident;
            quote!(#id)
        }
    });
    quote!( #(#parts),* )
}

/// Wrap a comma-list in angle brackets, or empty if the list is empty.
pub(crate) fn wrap_angle(inner: &TokenStream) -> TokenStream {
    if inner.is_empty() {
        quote!()
    } else {
        quote!( < #inner > )
    }
}
