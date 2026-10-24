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
    /// Traits **declared in this module**, keyed the same way an impl is (`impl_trait_key`).
    ///
    /// A declaration is what makes an impl's signature interpretable: the trait says which positions
    /// are `Self`, and an impl is free to spell those with the concrete self type instead (legal
    /// Rust). The terminator impl is of the SAME trait for a different self type, so those positions
    /// must be re-spelled `Self` before codegen — see [`canonicalize_method_sig`]. Only the
    /// declaration can drive that rewrite: a trait that genuinely declares a concrete `&A` parameter
    /// must be left alone.
    pub traits: BTreeMap<String, syn::ItemTrait>,
    /// Per-expansion nonce suffixed onto every generated identifier for hygiene.
    pub nonce: u64,
}

impl Model {
    /// The declaration of `name` in trait `trait_key`, when that trait is declared in this module.
    pub fn trait_method_sig(&self, trait_key: &str, name: &Ident) -> Option<&syn::Signature> {
        self.traits
            .get(trait_key)?
            .items
            .iter()
            .find_map(|it| match it {
                syn::TraitItem::Fn(f) if f.sig.ident == *name => Some(&f.sig),
                _ => None,
            })
    }

    /// Is this trait declared in the module (as opposed to `#[decycle] use`-imported)?
    pub fn has_trait_decl(&self, trait_key: &str) -> bool {
        self.traits.contains_key(trait_key)
    }

    pub fn collect(items: &[Item], nonce: u64) -> syn::Result<Self> {
        let mut adts = BTreeMap::new();
        let mut impls = Vec::new();
        let mut traits = BTreeMap::new();
        for it in items {
            match it {
                Item::Struct(ItemStruct {
                    ident, generics, ..
                })
                | Item::Enum(ItemEnum {
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
                Item::Trait(t) => {
                    traits.insert(t.ident.to_string(), t.clone());
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
            traits,
            nonce,
        })
    }
}

/// The trait's ident, used as the coherence key — only for a trait path of one segment (after a
/// leading `self::`). A qualified path (`crate::m::Tr`, `super::Tr`) is the caller's opt-out, as in
/// the ranked engine: the impl has no key, takes no part in the cycle and is re-emitted as written.
pub(crate) fn impl_trait_key(im: &ItemImpl) -> Option<String> {
    let (_, path, _) = im.trait_.as_ref()?;
    trait_key(path)
}

/// The key of the trait of a bound, as [`impl_trait_key`] keys an impl's: a bound naming the trait
/// by a path is the opt-out, so it is no edge of the cycle and is left as written.
pub(crate) fn trait_key(path: &syn::Path) -> Option<String> {
    bare_ident(path).map(|ident| ident.to_string())
}

/// The ident of the impl's self type, which keys it as a cycle member — only for a bare (or
/// `self::`) path: `impl Tr for crate::other::B` names the item itself, the opt-out, and is no
/// member even beside a local `B`.
pub(crate) fn impl_self_ident(im: &ItemImpl) -> Option<Ident> {
    crate::names::local_type_head_ident(&im.self_ty)
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
