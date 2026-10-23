//! The checks both engines make of a `#[decycle]` module's markers, up front and with the same
//! errors: a marker on a trait alias, a glob `#[decycle] use`, and a `#[decycle]` anywhere inside
//! a nested module are rejected (through `proc_macro_error`).

use crate::names::{use_leaves, UseLeaf};
use proc_macro_error::abort;
use syn::visit::Visit;
use syn::{Ident, ItemMod, ItemTraitAlias, UseTree};

/// Reject `#[decycle]` on a trait alias. An alias has no body to take part in a cycle (the ranked
/// engine has no `ItemTrait` to carry through its macro ping-pong): the marker produced a bogus
/// working-list entry in the ranked engine, and was left on the alias by the structural one.
pub(crate) fn reject_trait_alias(alias: &ItemTraitAlias) -> ! {
    abort!(&alias.ident, "#[decycle] is not supported on a trait alias")
}

/// The names a `#[decycle] use` routes, under their local names, in source order. A glob names
/// none, so it cannot say which traits are the cycle participants, and is rejected: an explicit
/// `use path::Trait;` is required.
pub(crate) fn routed_use_names(tree: &UseTree) -> Vec<&Ident> {
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
