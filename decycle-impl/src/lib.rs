use syn::*;

pub mod finalize;
mod process_module;
mod process_trait;

pub use proc_macro_error;
pub use type_leak;

pub use process_module::process_module;
pub use process_trait::process_trait;

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
