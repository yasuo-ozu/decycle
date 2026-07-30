//! Collection: read the module body into a [`Model`] of local ADTs and their trait impls.

use super::*;

/// A local algebraic data type (struct or enum) defined in the module.
pub(crate) struct Adt {
    pub ident: Ident,
    pub generics: syn::Generics,
    /// The full item, used to clone its shape when building the `Rec` mirror.
    pub item: Item,
}

impl Adt {
    /// The natural type's visibility — the generated `__MTerm` copies it so a private cycle type is
    /// wrapped by a private terminator (no `pub`-over-private `private_interfaces` mismatch).
    pub fn vis(&self) -> &syn::Visibility {
        match &self.item {
            Item::Struct(s) => &s.vis,
            Item::Enum(e) => &e.vis,
            _ => &syn::Visibility::Inherited,
        }
    }

    /// The type's own attributes (so its `#[cfg]`s can be replicated onto the generated terminator
    /// and impls — see [`crate::extract_cfg_attrs`]).
    pub fn attrs(&self) -> &[syn::Attribute] {
        match &self.item {
            Item::Struct(s) => &s.attrs,
            Item::Enum(e) => &e.attrs,
            _ => &[],
        }
    }
}

/// One `impl Trait for Type { .. }` block in the module.
pub(crate) struct ImplBlock {
    pub trait_key: String,
    pub self_ident: Ident,
    /// The generic args on the self type (`Statement<Span>` → `[Span]`).
    pub self_args: Vec<GenericArgument>,
    pub item: ItemImpl,
}

pub(crate) struct Model {
    pub adts: BTreeMap<String, Adt>,
    pub impls: Vec<ImplBlock>,
    /// Per-expansion nonce suffixed onto every generated identifier for hygiene.
    pub nonce: u64,
}

impl Model {
    pub fn collect(items: &[Item], nonce: u64) -> syn::Result<Self> {
        let mut adts = BTreeMap::new();
        let mut impls = Vec::new();
        for it in items {
            match it {
                Item::Struct(ItemStruct {
                    ident, generics, ..
                }) => {
                    adts.insert(
                        ident.to_string(),
                        Adt {
                            ident: ident.clone(),
                            generics: generics.clone(),
                            item: it.clone(),
                        },
                    );
                }
                Item::Enum(ItemEnum {
                    ident, generics, ..
                }) => {
                    adts.insert(
                        ident.to_string(),
                        Adt {
                            ident: ident.clone(),
                            generics: generics.clone(),
                            item: it.clone(),
                        },
                    );
                }
                Item::Impl(im) if im.trait_.is_some() => {
                    if let (Some(key), Some(self_id)) = (impl_trait_key(im), impl_self_ident(im)) {
                        impls.push(ImplBlock {
                            trait_key: key,
                            self_ident: self_id,
                            self_args: impl_self_args(im),
                            item: im.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(Model {
            adts,
            impls,
            nonce,
        })
    }
}

/// The trait's last-segment ident, used as the coherence key (`a::Foo` and `b::Foo` collide — same as
/// the visitor system's last-segment keying).
pub(crate) fn impl_trait_key(im: &ItemImpl) -> Option<String> {
    im.trait_
        .as_ref()
        .and_then(|(_, path, _)| path.segments.last())
        .map(|s| s.ident.to_string())
}

pub(crate) fn impl_self_ident(im: &ItemImpl) -> Option<Ident> {
    if let Type::Path(tp) = &*im.self_ty {
        tp.path.segments.last().map(|s| s.ident.clone())
    } else {
        None
    }
}

fn impl_self_args(im: &ItemImpl) -> Vec<GenericArgument> {
    if let Type::Path(tp) = &*im.self_ty {
        if let Some(seg) = tp.path.segments.last() {
            if let PathArguments::AngleBracketed(ab) = &seg.arguments {
                return ab.args.iter().cloned().collect();
            }
        }
    }
    vec![]
}
