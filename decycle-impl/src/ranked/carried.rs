//! Traits that reach a `#[decycle] use` site through an [announce](https://docs.rs/announce)
//! trampoline.
//!
//! A trait is made available to `#[decycle] use` by announcing it where it is defined:
//! `#[announce(crate = ::decycle::announce)]`, optionally with `mock = path` (a stand-in for the real
//! trait at `path`) and `decycle::check` (definition-side validation). The trait travels as
//! announce carries any item; this module is the receiving end.

use proc_macro2::Span;
use proc_macro_error::*;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;
use syn::*;

/// Receive one trampoline entry at `__finalize`: the announced trait, its generic parameters
/// renamed to decycle's randomized names.
///
/// An entry announced with a `repeater` is rejected: decycle does not read the repeater table, so
/// a relative type in the trait would reach the module unresolved.
///
/// The module's `impl Tr for X` is matched against the carried trait by NAME, so the carried trait's
/// ident becomes the name the module spells it by:
///
/// - `#[decycle] use path::Tr;` names it by the trampoline's name, which must be the trait's own
///   (the name of the item, or, for an entry with a `mock(path)` META — a stand-in announced with
///   `#[announce(mock = path)]`, such as announce-std's — `path`'s last segment: the stand-in is the
///   real trait). A trampoline named otherwise (`macro_name = N`, or a stand-in with a name of its
///   own) is an error unless it is renamed;
/// - `#[decycle] use path::N as Local;` names it `Local` (`renames`: `(original, local)` pairs), and
///   then `Local` is what the module's impls spell, whatever the trait is called.
///
/// An entry with a `mock(path)` META also records an absolute `path` (see [`MOCK_ATTR`]) for the
/// places that name the trait itself.
pub(crate) fn receive_carried_trait(
    entry: &announce::helper::Entry,
    renames: &[(Ident, Ident)],
) -> ItemTrait {
    let mut trait_item: ItemTrait = match syn::parse2::<Item>(entry.item.clone()) {
        Ok(Item::Trait(t)) => t,
        _ => abort!(
            entry.item.span(),
            "#[decycle] use: `{}` is not a trait",
            template_quote::ToTokens::to_token_stream(&entry.key).to_string()
        ),
    };
    if entry.find_meta("type_leak").is_some() {
        abort!(
            entry.item.span(),
            "#[decycle] use: `{}` is announced with a `repeater`, which decycle does not support",
            template_quote::ToTokens::to_token_stream(&entry.key).to_string();
            help = "announce the trait without `repeater` and spell its types with absolute paths (`crate::..`, `::krate::..`)"
        )
    }
    let mut renamer = crate::randomize_impl_generics(&mut trait_item.generics, crate::get_random());
    renamer.visit_item_trait_mut(&mut trait_item);
    let mock = entry.mock();
    // The trait's own name: what the module's impls spell, when the trampoline is used as it is.
    let own = match &mock {
        Some(path) => path
            .segments
            .last()
            .unwrap_or_else(|| abort!(path, "decycle: empty `mock` path"))
            .ident
            .clone(),
        None => trait_item.ident.clone(),
    };
    // The trampoline's name in the module: the local name of the `#[decycle] use` (its alias, if it
    // has an `as`), which is what the continuation list holds.
    if let Some(used) = entry.key.path().segments.last().map(|seg| seg.ident.clone()) {
        trait_item.ident = match renames.iter().find(|(_, local)| *local == used) {
            // `use path::N as Local;`: the module spells the trait `Local`.
            Some((_, local)) => local.clone(),
            None if used == own => own,
            None => {
                let (what, keep) = if mock.is_some() {
                    (
                        format!("the stand-in `{}` is named differently from the trait `{}` it stands in for", used, own),
                        format!("keep the real trait `{}` in scope", own),
                    )
                } else {
                    (
                        format!("the trampoline `{}` is named differently from the trait `{}` it carries (`macro_name = {}`)", used, own, used),
                        format!("also `use` the trait `{}` itself, which the trampoline's name does not import", own),
                    )
                };
                abort!(
                    entry.key.span(),
                    "#[decycle] use: {}, and is not renamed", what;
                    help = "the module's `impl {} for ..` is matched to the trait by name, so give the `use` that name: `#[decycle] use path::to::{} as {};`, and {}", own, used, own, keep
                )
            }
        };
    } else {
        trait_item.ident = own;
    }
    if let Some(path) = mock {
        if is_absolute(&path) {
            let attr = Ident::new(MOCK_ATTR, Span::call_site());
            trait_item.attrs.push(parse_quote!(#[#attr(#path)]));
        }
    }
    trait_item
}

/// The name of the inert attribute [`receive_carried_trait`] puts on a carried stand-in trait to
/// record its real trait's absolute path. Carried traits' own attributes are never re-emitted by
/// `finalize`, so it only travels inside the `ItemTrait`; read it with [`mock_path`].
pub(crate) const MOCK_ATTR: &str = "__decycle_mock";

/// The real trait's absolute path of a carried stand-in trait (`mock(path)` META), if any.
pub(crate) fn mock_path(trait_item: &ItemTrait) -> Option<Path> {
    trait_item
        .attrs
        .iter()
        .find(|a| a.path().is_ident(MOCK_ATTR))
        .and_then(|a| a.parse_args::<Path>().ok())
}

/// `::a::b`, `crate::..` or `$crate::..` (what announce turns the definer's `crate::` into): a
/// path that names the same item from any module.
fn is_absolute(path: &Path) -> bool {
    path.leading_colon.is_some()
        || path
            .segments
            .first()
            .is_some_and(|s| s.ident == "crate" || s.ident == "$crate")
}
