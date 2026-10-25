//! A `#[decycle]` module's markers, as both engines take them: every marker of a trait, trait
//! alias or `use` is taken off ([`take_marked`]), and the module is checked up front, with the same
//! errors in both: a marker on a trait alias, a glob `#[decycle] use`, and a `#[decycle]` anywhere
//! inside a nested module are rejected (through `proc_macro_error`).

use crate::names::{use_leaves, UseLeaf};
use proc_macro_error::abort;
use syn::visit::Visit;
use syn::{Attribute, Ident, Item, ItemMod, ItemTrait, ItemTraitAlias, ItemUse, UseTree};

/// A marked item of a `#[decycle]` module, its markers taken ([`take_marked`]).
pub(crate) enum Marked<'a> {
    /// `#[decycle] trait Tr { .. }`.
    Trait(&'a ItemTrait),
    /// `#[decycle] use path::{Tr, Other as O};`, with the names it routes ([`routed_use_names`]).
    Use(&'a mut ItemUse, Vec<Ident>),
}

/// Take the `#[decycle]` markers off `items`' traits, trait aliases and `use`s, and return the
/// marked ones, in order. Every marker of an item is taken, and each is checked
/// (`crate::is_decycle_attribute` rejects an argument list): a marker is a flag, so a second one,
/// which a macro marking an item the user marked already can produce, says nothing more. A marked
/// trait alias or glob `use` is rejected, at the item, in source order.
pub(crate) fn take_marked<'a>(items: &'a mut [Item], decycle_crate: &Ident) -> Vec<Marked<'a>> {
    let mut out = Vec::new();
    for item in items.iter_mut() {
        match item {
            Item::Trait(t) => {
                if take_markers(&mut t.attrs, decycle_crate) {
                    out.push(Marked::Trait(t));
                }
            }
            Item::TraitAlias(a) => {
                if take_markers(&mut a.attrs, decycle_crate) {
                    reject_trait_alias(a)
                }
            }
            Item::Use(u) => {
                if take_markers(&mut u.attrs, decycle_crate) {
                    let names = routed_use_names(&u.tree).into_iter().cloned().collect();
                    out.push(Marked::Use(u, names));
                }
            }
            _ => {}
        }
    }
    out
}

/// Remove every `#[decycle]` marker from `attrs`; whether there was one.
fn take_markers(attrs: &mut Vec<Attribute>, decycle_crate: &Ident) -> bool {
    let before = attrs.len();
    attrs.retain(|a| !crate::is_decycle_attribute(a, decycle_crate));
    attrs.len() != before
}

/// Reject `#[decycle]` on a trait alias. An alias has no body to take part in a cycle (the ranked
/// engine has no `ItemTrait` to carry through its macro ping-pong): the marker produced a bogus
/// working-list entry in the ranked engine, and was left on the alias by the structural one.
fn reject_trait_alias(alias: &ItemTraitAlias) -> ! {
    abort!(&alias.ident, "#[decycle] is not supported on a trait alias")
}

/// The names a `#[decycle] use` routes, under their local names, in source order. A glob names
/// none, so it cannot say which traits are the cycle participants, and is rejected: an explicit
/// `use path::Trait;` is required.
fn routed_use_names(tree: &UseTree) -> Vec<&Ident> {
    use_leaves(tree)
        .into_iter()
        .map(|leaf| match leaf {
            UseLeaf::Name(ident) | UseLeaf::Rename { local: ident, .. } => ident,
            UseLeaf::Glob(glob) => abort!(glob, "glob is not supported in #[decycle] use"),
        })
        .collect()
}

/// Reject a `#[decycle]` anywhere inside `module`, a module nested in a `#[decycle]` module: on the
/// nested module itself or on any item in it. The ranked engine re-emits the module's items in
/// modules of its own, where a nested one would no longer mean what it says, and neither engine
/// reads a nested module's items.
pub(crate) fn check_submodule(module: &ItemMod, decycle_crate: &Ident) {
    struct Visitor<'a> {
        decycle_crate: &'a Ident,
    }
    impl<'ast> Visit<'ast> for Visitor<'_> {
        fn visit_attribute(&mut self, i: &'ast syn::Attribute) {
            if crate::is_decycle_attribute(i, self.decycle_crate) {
                abort!(&i, "#[decycle] is not supported in nested modules")
            }
            syn::visit::visit_attribute(self, i);
        }
    }
    Visitor { decycle_crate }.visit_item_mod(module);
}
