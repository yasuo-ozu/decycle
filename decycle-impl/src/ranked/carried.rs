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

/// The carried item of one trampoline entry, as a trait. An item that is not a trait is rejected at
/// the user's `#[decycle] use` (`entry.key`), not at the definition, which may be in another crate.
fn entry_trait(entry: &announce::helper::Entry) -> ItemTrait {
    match syn::parse2::<Item>(entry.item.clone()) {
        Ok(Item::Trait(t)) => t,
        _ => abort!(
            entry.key.span(),
            "#[decycle] use: `{}` is not a trait",
            template_quote::ToTokens::to_token_stream(&entry.key).to_string()
        ),
    }
}

/// The name the module spells a carried trait by: what its `impl Tr for X` and bounds say.
///
/// The module's impls are matched against the carried trait by NAME, and the module spells the trait
/// by the local name of its `#[decycle] use`: the last segment of the path (`#[decycle] use path::Tr;`
/// names it `Tr`), or the alias if it has an `as` (`#[decycle] use path::Tr as Local;` names it
/// `Local`, whatever the trait is called). `renames` holds the `(original, local)` pairs of the
/// module's `as`es.
///
/// A stand-in (an entry with a `mock(path)` META, announced with `#[announce(mock = path)]`, such as
/// announce-std's) is the real trait at `path`, so the name it is reached under has to be `path`'s
/// last segment, or an alias given by an `as`. Otherwise the module's impls are not the stand-in's,
/// and that is an error.
fn carried_name(
    entry: &announce::helper::Entry,
    item_ident: &Ident,
    renames: &[(Ident, Ident)],
) -> Ident {
    let mock = entry.mock();
    let Some(used) = entry.key.path().segments.last().map(|seg| seg.ident.clone()) else {
        return item_ident.clone();
    };
    let Some(path) = mock else {
        return used;
    };
    let own = &path
        .segments
        .last()
        .unwrap_or_else(|| abort!(path, "decycle: empty `mock` path"))
        .ident;
    if used == *own || renames.iter().any(|(_, local)| *local == used) {
        return used;
    }
    abort!(
        entry.key.span(),
        "#[decycle] use: the stand-in `{}` is reached under a name other than `{}`, the trait it stands in for, and is not renamed", used, own;
        help = "the module's `impl {} for ..` is matched to the trait by name, so give the `use` that name: `#[decycle] use path::to::{} as {};` (the real trait must be in scope under that name too, unless the stand-in imports it)", own, used, own
    )
}

/// Receive one trampoline entry at `__finalize`: the announced trait, its generic parameters
/// renamed to decycle's randomized names, and named as the module spells it (see [`carried_name`]).
///
/// The trait is marked ([`CARRIED_ATTR`]): its `#[decycle] use ... as` rename is applied here, once.
/// An entry with a `mock(path)` META also records an absolute `path` (see [`MOCK_ATTR`]) for the
/// places that name the trait itself.
pub(crate) fn receive_carried_trait(
    entry: &announce::helper::Entry,
    renames: &[(Ident, Ident)],
) -> ItemTrait {
    let mut trait_item = entry_trait(entry);
    let mut renamer = crate::randomize_impl_generics(&mut trait_item.generics, crate::get_random());
    renamer.visit_item_trait_mut(&mut trait_item);
    trait_item.ident = carried_name(entry, &trait_item.ident, renames);
    if let Some(path) = entry.mock() {
        if is_absolute(&path) {
            let attr = Ident::new(MOCK_ATTR, Span::call_site());
            trait_item.attrs.push(parse_quote!(#[#attr(#path)]));
        }
    }
    let attr = Ident::new(CARRIED_ATTR, Span::call_site());
    trait_item.attrs.push(parse_quote!(#[#attr]));
    trait_item
}

/// The name of the inert attribute [`receive_carried_trait`] puts on every carried trait, saying
/// that the `#[decycle] use ... as` rename of its entry has been applied to it. Read it with
/// [`is_carried`].
pub(crate) const CARRIED_ATTR: &str = "__decycle_carried";

/// Whether `trait_item` was received from a trampoline ([`receive_carried_trait`]) and so already
/// bears the name its module spells.
pub(crate) fn is_carried(trait_item: &ItemTrait) -> bool {
    trait_item
        .attrs
        .iter()
        .any(|a| a.path().is_ident(CARRIED_ATTR))
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
