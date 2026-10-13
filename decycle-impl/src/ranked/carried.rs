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
/// An entry with a `mock(path)` META (a stand-in announced with `#[announce(mock = path)]`, such as
/// announce-std's) is the real trait at `path`: its ident
/// becomes `path`'s last segment, so the module's `impl RealTrait for X` is matched, and an
/// absolute `path` is recorded (see [`MOCK_ATTR`]) for the places that name the trait itself.
pub(crate) fn receive_carried_trait(entry: &announce::helper::Entry) -> ItemTrait {
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
    if let Some(path) = entry.mock() {
        let real = path
            .segments
            .last()
            .unwrap_or_else(|| abort!(path, "decycle: empty `mock` path"));
        trait_item.ident = real.ident.clone();
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
