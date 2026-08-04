//! Defuse the two name-resolution hazards created by *this crate's own* output.
//!
//! `finalize` re-emits every adopted impl inside helper modules nested one or two levels below the
//! module the user wrote (`shadowing_module`, `shadowing_module::ranked_traits`), and those modules
//! glob-import both enclosing scopes (`use super::*;` plus `use super::super::*;`). Two things break
//! as a result, and neither is the caller's fault:
//!
//! 1. **Ambiguity (`E0659`).** A bare name reachable through *both* globs — which is exactly the
//!    situation for every type defined in the processed module, since it is visible one level up as
//!    well — becomes ambiguous at the use site.
//! 2. **Relative paths change meaning.** `super::Marker` written in a where-clause means "one level
//!    above the module I am written in". After nesting it points one or two levels too shallow, so it
//!    silently resolves elsewhere or not at all.
//!
//! Both are fixed the same way: resolve the name *once*, at the level where the caller wrote it, bind
//! the result to an unambiguous alias, and refer to the alias from inside the nested impls. The
//! aliases are emitted as ordinary `use` items in the processed module, where they resolve exactly as
//! the original spelling did; the nesting then cannot change what they mean.
//!
//! Alias names are **deterministic** (they carry the module's own ident, not a random nonce), so
//! diagnostics that mention one are stable enough to match in a golden test.

use proc_macro2::Ident;
use syn::spanned::Spanned;
use std::collections::HashSet;
use syn::visit_mut::VisitMut;
use syn::{Item, ItemImpl, Path, PathArguments, PathSegment, TraitBound, TypePath};
use template_quote::quote;

/// Rewrite `adopted` so no bare cycle-type name and no relative path survives into the nested
/// modules, returning the `use` items that define the aliases introduced.
///
/// `local_cycle_heads` are the cycle types **defined in this module** — only those can be ambiguous
/// under the double glob, and only those can be named by a `use self::…` alias. A foreign type with a
/// cyclic impl here (`impl Tr for &str`) is reachable by exactly one path and needs no alias; trying
/// to bind one would be `E0432: unresolved import self::str`.
pub(crate) fn defuse_nesting(
    adopted: &mut [ItemImpl],
    local_cycle_heads: &HashSet<Ident>,
    cyclic_traits: &HashSet<Ident>,
    module_ident: &Ident,
) -> Vec<Item> {
    let mut items = Vec::new();

    // (1) Bind each locally-defined cycle type to a name only *this* module introduces.
    let mut heads: Vec<&Ident> = local_cycle_heads.iter().collect();
    heads.sort_by_key(|i| i.to_string());
    let aliases: Vec<(String, String)> = heads
        .iter()
        .map(|head| {
            let name = format!("__DecycleNat_{head}_{module_ident}");
            let alias = Ident::new(&name, head.span());
            items.push(syn::parse_quote! {
                #[allow(non_camel_case_types, unused_imports)]
                use self::#head as #alias;
            });
            (head.to_string(), name)
        })
        .collect();
    for im in adopted.iter_mut() {
        AliasHeads(&aliases).visit_item_impl_mut(im);
    }

    // (2) Lift every `super::…` / `self::…` path to an alias resolved at this level.
    let mut lifted: Vec<(Ident, Path)> = Vec::new();
    for im in adopted.iter_mut() {
        LiftRelative {
            lifted: &mut lifted,
            cyclic_traits,
            module_ident,
        }
        .visit_item_impl_mut(im);
    }
    for (alias, path) in &lifted {
        items.push(syn::parse_quote! {
            #[allow(non_camel_case_types, unused_imports)]
            use #path as #alias;
        });
    }

    items
}

/// Re-spell the leading segment of any non-`::`-rooted path that names a local cycle type.
///
/// The replacement ident is stamped with the span of the ident it replaces, not `call_site` — an
/// alias is an internal detail, and a diagnostic mentioning one should still point at the line the
/// caller wrote rather than at the `#[decycle]` attribute.
struct AliasHeads<'a>(&'a [(String, String)]);

impl VisitMut for AliasHeads<'_> {
    fn visit_path_mut(&mut self, path: &mut Path) {
        if path.leading_colon.is_none() {
            if let Some(first) = path.segments.first_mut() {
                let name = first.ident.to_string();
                if let Some((_, alias)) = self.0.iter().find(|(n, _)| *n == name) {
                    first.ident = Ident::new(alias, first.ident.span());
                }
            }
        }
        syn::visit_mut::visit_path_mut(self, path);
    }
}

/// Replace a `super::…` / `self::…` rooted path with a single-segment alias, recording the original
/// so the caller can bind it. Generic arguments stay on the alias (`super::Wrap<T>` ⇒ `Alias<T>`),
/// since only the *path* needs resolving, not the instantiation.
///
/// Deliberately narrow — three kinds of path are left exactly as written:
/// - **anything naming a cyclic trait.** A relative trait reference is decycle's own signal, already
///   normalised by `strip_leading_self`; aliasing it to a one-segment name would make an
///   ordinary premise look like a cycle edge (or vice versa).
/// - **qualified paths** (`<B as self::Tr>::Assoc`). Their `qself.position` indexes into `segments`,
///   so collapsing the path to a single segment corrupts the syntax tree.
/// - **expressions.** Only types and trait bounds are visited; a body path is left to resolve
///   normally.
struct LiftRelative<'a> {
    lifted: &'a mut Vec<(Ident, Path)>,
    cyclic_traits: &'a HashSet<Ident>,
    module_ident: &'a Ident,
}

impl LiftRelative<'_> {
    fn lift(&mut self, path: &mut Path) {
        if path.leading_colon.is_some() || path.segments.len() < 2 {
            return;
        }
        let root = path.segments[0].ident.to_string();
        if root != "super" && root != "self" {
            return;
        }
        let mut bare = path.clone();
        let args = std::mem::replace(
            &mut bare.segments.last_mut().unwrap().arguments,
            PathArguments::None,
        );
        let key = quote!(#bare).to_string();
        let span = path.segments[0].ident.span();
        let alias = match self
            .lifted
            .iter()
            .find(|(_, p)| quote!(#p).to_string() == key)
        {
            // Re-stamped with THIS site's span — see `AliasHeads`.
            Some((a, _)) => Ident::new(&a.to_string(), span),
            None => {
                let a = Ident::new(
                    &format!(
                        "__DecycleRelPath_{}_{}",
                        self.lifted.len(),
                        self.module_ident
                    ),
                    span,
                );
                self.lifted.push((a.clone(), bare));
                a
            }
        };
        *path = Path {
            leading_colon: None,
            segments: std::iter::once(PathSegment {
                ident: alias,
                arguments: args,
            })
            .collect(),
        };
    }

    fn names_cyclic_trait(&self, path: &Path) -> bool {
        path.segments
            .last()
            .is_some_and(|s| self.cyclic_traits.contains(&s.ident))
    }
}

impl VisitMut for LiftRelative<'_> {
    fn visit_type_path_mut(&mut self, tp: &mut TypePath) {
        // Depth-first, so an inner relative argument is lifted before its enclosing path.
        syn::visit_mut::visit_type_path_mut(self, tp);
        if tp.qself.is_some() || self.names_cyclic_trait(&tp.path) {
            return;
        }
        self.lift(&mut tp.path);
    }

    fn visit_trait_bound_mut(&mut self, tb: &mut TraitBound) {
        syn::visit_mut::visit_trait_bound_mut(self, tb);
        if self.names_cyclic_trait(&tb.path) {
            return;
        }
        self.lift(&mut tb.path);
    }
}
