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
/// The module's impls are matched against the carried trait by NAME. The trampoline is reached under
/// a name (`used`, the last segment of the `#[decycle] use`, its alias if it has an `as`) that is not
/// always the trait's:
///
/// - `#[decycle] use path::N as Local;` (`renames`: `(original, local)` pairs): the module spells the
///   trait `Local`, whatever it is called;
/// - `#[decycle] use path::Tr;` with `used` equal to the trait's own name: the module spells it so.
///   The own name is the name of the item, or, for an entry with a `mock(path)` META (a stand-in
///   announced with `#[announce(mock = path)]`, such as announce-std's), `path`'s last segment: the
///   stand-in IS the real trait;
/// - `used` differs from the own name, the entry is not a stand-in: the trampoline is reached under
///   another name (`macro_name = N`, or a re-export under an alias). The module spells the trait by
///   its own name, which a plain `use` of the trait supplies. The `use` is not renamed because a
///   renaming `use` would import every namespace of the name, and `Tr` may be taken in the macro
///   namespace by a derive or attribute macro;
/// - `used` differs from the own name, the entry is a stand-in: an error. A stand-in is not the
///   trait, so the module's impls are the real trait's only if the `use` says so, with an `as`.
pub(crate) fn carried_name(
    entry: &announce::helper::Entry,
    item_ident: &Ident,
    renames: &[(Ident, Ident)],
) -> Ident {
    let mock = entry.mock();
    // The trait's own name: what the module's impls spell, when the trampoline is used as it is.
    let own = match &mock {
        Some(path) => path
            .segments
            .last()
            .unwrap_or_else(|| abort!(path, "decycle: empty `mock` path"))
            .ident
            .clone(),
        None => item_ident.clone(),
    };
    let Some(used) = entry.key.path().segments.last().map(|seg| seg.ident.clone()) else {
        return own;
    };
    match renames.iter().find(|(_, local)| *local == used) {
        Some((_, local)) => local.clone(),
        None if used == own || mock.is_none() => own,
        None => abort!(
            entry.key.span(),
            "#[decycle] use: the stand-in `{}` is reached under a name other than `{}`, the trait it stands in for, and is not renamed", used, own;
            help = "the module's `impl {} for ..` is matched to the trait by name, so give the `use` that name: `#[decycle] use path::to::{} as {};` (the real trait must be in scope under that name too, unless the stand-in imports it)", own, used, own
        ),
    }
}

/// [`carried_name`] of an entry whose trait has not been parsed yet.
pub(crate) fn entry_name(entry: &announce::helper::Entry, renames: &[(Ident, Ident)]) -> Ident {
    let trait_item = entry_trait(entry);
    carried_name(entry, &trait_item.ident, renames)
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
    let used = entry.key.path().segments.last().map(|seg| seg.ident.clone());
    if let Some(path) = entry.mock() {
        if is_absolute(&path) {
            let attr = Ident::new(MOCK_ATTR, Span::call_site());
            trait_item.attrs.push(parse_quote!(#[#attr(#path)]));
        }
    }
    let attr = Ident::new(CARRIED_ATTR, Span::call_site());
    match used {
        Some(used) => trait_item.attrs.push(parse_quote!(#[#attr(#used)])),
        None => trait_item.attrs.push(parse_quote!(#[#attr])),
    }
    trait_item
}

/// The name of the inert attribute [`receive_carried_trait`] puts on every carried trait, saying
/// that the `#[decycle] use ... as` rename of its entry has been applied to it, and holding the
/// name the trampoline was reached under. Read it with [`is_carried`] and [`carried_via`].
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

/// The name a carried trait's trampoline was reached under (the last segment of the module's
/// `#[decycle] use`, its alias if it has an `as`). That `use` imports every namespace of the name,
/// and the trait may be one of them: a re-export (`pub use d::Tr as Step;`) imports the trait itself
/// as `Step` as well as its trampoline.
pub(crate) fn carried_via(trait_item: &ItemTrait) -> Option<Ident> {
    trait_item
        .attrs
        .iter()
        .find(|a| a.path().is_ident(CARRIED_ATTR))
        .and_then(|a| a.parse_args::<Ident>().ok())
}

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
