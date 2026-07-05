// `HashMap`/`VisitMut` are used solely by the `GenericRenamer` cluster below, which only
// the type-leak-gated `process_trait` path calls — gate the imports to keep the
// `default-features = false` (finalize-only) build warning-free.
#[cfg(feature = "type-leak")]
use std::collections::HashMap;
#[cfg(feature = "type-leak")]
use syn::visit_mut::VisitMut;
use syn::*;

pub mod finalize;
mod helper;
mod process_module;
#[cfg(feature = "type-leak")]
mod process_trait;

pub use proc_macro_error;
#[cfg(feature = "type-leak")]
pub use type_leak;

pub use process_module::process_module;
#[cfg(feature = "type-leak")]
pub use process_trait::process_trait;

#[cfg(feature = "type-leak")]
#[derive(Clone)]
struct GenericRenamer {
    pub(crate) lifetime_renames: HashMap<String, Lifetime>,
    pub(crate) ident_renames: HashMap<String, Ident>,
}

#[cfg(feature = "type-leak")]
impl VisitMut for GenericRenamer {
    fn visit_lifetime_mut(&mut self, lt: &mut Lifetime) {
        if let Some(new) = self.lifetime_renames.get(&lt.ident.to_string()) {
            *lt = new.clone();
            return;
        }
        syn::visit_mut::visit_lifetime_mut(self, lt);
    }

    fn visit_type_mut(&mut self, ty: &mut Type) {
        if let Type::Path(type_path) = ty {
            if type_path.qself.is_none() && type_path.path.segments.len() == 1 {
                let segment = &mut type_path.path.segments[0];
                if matches!(segment.arguments, PathArguments::None) {
                    if let Some(new) = self.ident_renames.get(&segment.ident.to_string()) {
                        segment.ident = new.clone();
                    }
                }
            }
        }
        syn::visit_mut::visit_type_mut(self, ty);
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        if let Expr::Path(expr_path) = expr {
            if expr_path.qself.is_none() && expr_path.path.segments.len() == 1 {
                let segment = &mut expr_path.path.segments[0];
                if matches!(segment.arguments, PathArguments::None) {
                    if let Some(new) = self.ident_renames.get(&segment.ident.to_string()) {
                        segment.ident = new.clone();
                    }
                }
            }
        }
        syn::visit_mut::visit_expr_mut(self, expr);
    }
}

#[cfg(feature = "type-leak")]
pub(crate) fn randomize_impl_generics(
    generics: &mut Generics,
    random_suffix: u64,
) -> GenericRenamer {
    let mut lifetime_renames: HashMap<String, Lifetime> = HashMap::new();
    let mut ident_renames: HashMap<String, Ident> = HashMap::new();

    for param in &mut generics.params {
        match param {
            GenericParam::Lifetime(lt) => {
                let old = lt.lifetime.clone();
                let new_name = format!("'{}{}", old.ident, random_suffix);
                let new = Lifetime::new(&new_name, old.span());
                lt.lifetime = new.clone();
                lifetime_renames.insert(old.ident.to_string(), new);
            }
            GenericParam::Type(tp) => {
                let old = tp.ident.clone();
                let new = Ident::new(&format!("{}{}", old, random_suffix), old.span());
                tp.ident = new.clone();
                ident_renames.insert(old.to_string(), new);
            }
            GenericParam::Const(cp) => {
                let old = cp.ident.clone();
                let new = Ident::new(&format!("{}{}", old, random_suffix), old.span());
                cp.ident = new.clone();
                ident_renames.insert(old.to_string(), new);
            }
        }
    }

    let mut renamer = GenericRenamer {
        lifetime_renames,
        ident_renames,
    };
    renamer.visit_generics_mut(generics);
    renamer
}

/// Recognize a `#[decycle]` attribute on an inner item — the bare `#[decycle]` or the two-segment
/// `#[<crate>::decycle]` form, where `<crate>` is the decycle crate name *as passed to the macro*
/// (`decycle_crate` — the leading segment of the `decycle = …` path argument, default `decycle`).
/// We deliberately do NOT read the consumer's `Cargo.toml` to discover a dependency rename: a renamed
/// decycle must be named explicitly via `#[decycle(decycle = ::my_rename)]`, and the two-segment inner
/// form is matched against that name. (Dropping the manifest read removes the `toml` dependency and
/// lowers the crate's MSRV.)
fn is_decycle_attribute(attr: &Attribute, decycle_crate: &Ident) -> bool {
    let path = attr.path();
    path.is_ident("decycle")
        || (path.segments.len() == 2
            && (path.segments[0].ident == "decycle" || &path.segments[0].ident == decycle_crate)
            && path.segments[1].ident == "decycle")
}

fn ident_to_path(ident: &Ident) -> Path {
    Path {
        leading_colon: None,
        segments: core::iter::once(PathSegment {
            ident: ident.clone(),
            arguments: PathArguments::None,
        })
        .collect(),
    }
}

fn get_random() -> u64 {
    identity_to_u64(&get_crate_identity())
}

fn get_crate_identity() -> String {
    "decycle".to_string()
}

fn identity_to_u64(value: &str) -> u64 {
    // Deterministic FNV-1a hash for stable crate-local randomness.
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
