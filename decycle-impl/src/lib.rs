use syn::*;
use syn::visit_mut::VisitMut;

pub mod finalize;
mod process_module;
mod process_trait;

pub use proc_macro_error;
pub use type_leak;

pub use process_module::process_module;
pub use process_trait::process_trait;

#[derive(Clone)]
pub(crate) struct GenericRenamer {
    pub(crate) lifetime_renames: Vec<(Lifetime, Lifetime)>,
    pub(crate) ident_renames: Vec<(Ident, Ident)>,
}

impl VisitMut for GenericRenamer {
    fn visit_lifetime_mut(&mut self, lt: &mut Lifetime) {
        for (old, new) in &self.lifetime_renames {
            if lt == old {
                *lt = new.clone();
                return;
            }
        }
        syn::visit_mut::visit_lifetime_mut(self, lt);
    }

    fn visit_ident_mut(&mut self, ident: &mut Ident) {
        for (old, new) in &self.ident_renames {
            if ident == old {
                *ident = new.clone();
                return;
            }
        }
        syn::visit_mut::visit_ident_mut(self, ident);
    }
}

pub(crate) fn randomize_impl_generics(
    generics: &mut Generics,
    random_suffix: u64,
) -> GenericRenamer {
    let mut lifetime_renames: Vec<(Lifetime, Lifetime)> = Vec::new();
    let mut ident_renames: Vec<(Ident, Ident)> = Vec::new();

    for param in &mut generics.params {
        match param {
            GenericParam::Lifetime(lt) => {
                let old = lt.lifetime.clone();
                let new_name = format!("'{}{}", old.ident, random_suffix);
                let new = Lifetime::new(&new_name, old.span());
                lt.lifetime = new.clone();
                lifetime_renames.push((old, new));
            }
            GenericParam::Type(tp) => {
                let old = tp.ident.clone();
                let new = Ident::new(&format!("{}{}", old, random_suffix), old.span());
                tp.ident = new.clone();
                ident_renames.push((old, new));
            }
            GenericParam::Const(cp) => {
                let old = cp.ident.clone();
                let new = Ident::new(&format!("{}{}", old, random_suffix), old.span());
                cp.ident = new.clone();
                ident_renames.push((old, new));
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

fn is_decycle_attribute(attr: &Attribute) -> bool {
    let path = &attr.path();
    path.is_ident("decycle")
        || (path.segments.len() == 2
            && (&path.segments[0].ident == "decycle"
                || is_renamed_decycle_crate(&path.segments[0].ident))
            && &path.segments[1].ident == "decycle")
}

fn is_renamed_decycle_crate(ident: &proc_macro2::Ident) -> bool {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();

    if let Ok(cargo_toml) = std::fs::read_to_string(format!("{}/Cargo.toml", manifest_dir)) {
        if cargo_toml.contains(&format!("{} = {{ package = \"decycle\"", ident))
            || cargo_toml.contains(&format!("{} = {{ package = 'decycle'", ident))
        {
            return true;
        }
    }

    false
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
