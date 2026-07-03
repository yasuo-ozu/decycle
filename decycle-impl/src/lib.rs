use std::collections::HashMap;
use syn::visit_mut::VisitMut;
use syn::*;

pub mod finalize;
mod helper;
mod process_module;
mod process_trait;

pub use proc_macro_error;
pub use type_leak;

pub use process_module::process_module;
pub use process_trait::process_trait;

#[derive(Clone)]
struct GenericRenamer {
    pub(crate) lifetime_renames: HashMap<String, Lifetime>,
    pub(crate) ident_renames: HashMap<String, Ident>,
}

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

    let Ok(cargo_toml) = std::fs::read_to_string(format!("{}/Cargo.toml", manifest_dir)) else {
        return false;
    };

    let ident_str = ident.to_string();
    // Fast pre-check: skip the TOML parse when the ident can't possibly be a dependency
    // key at all. The real match below never trusts textual proximity beyond this.
    if !cargo_toml.contains(&ident_str) {
        return false;
    }

    // `toml::Value`'s `FromStr` expects a single bare value, not a whole document (it
    // errors "unexpected content, expected nothing" on any real Cargo.toml) — the
    // top-level table needs `toml::Table`'s own `FromStr` instead, which is exactly what
    // `dependency_tables_of` is typed to walk.
    let Ok(doc) = cargo_toml.parse::<toml::Table>() else {
        return false;
    };

    let found = dependency_tables_of(&doc).any(|deps| dep_is_renamed_decycle(deps, &ident_str));
    found
}

/// A dependency spec `ident = { package = "decycle", ... }` (any inline-table key order,
/// since it's parsed rather than text-matched).
fn dep_is_renamed_decycle(deps: &toml::Table, ident_str: &str) -> bool {
    matches!(
        deps.get(ident_str),
        Some(toml::Value::Table(spec))
            if spec.get("package").and_then(toml::Value::as_str) == Some("decycle")
    )
}

/// Every `[dependencies]`-shaped table reachable from the manifest root: the three
/// top-level kinds, plus the same three nested under each `[target.'cfg(...)'.*]`.
fn dependency_tables_of(doc: &toml::Table) -> impl Iterator<Item = &toml::Table> {
    const DEP_KEYS: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];

    let top_level = DEP_KEYS.iter().filter_map(|key| match doc.get(*key) {
        Some(toml::Value::Table(t)) => Some(t),
        _ => None,
    });

    let per_target = match doc.get("target") {
        Some(toml::Value::Table(targets)) => Some(targets),
        _ => None,
    }
    .into_iter()
    .flat_map(|targets| targets.values())
    .filter_map(|v| match v {
        toml::Value::Table(t) => Some(t),
        _ => None,
    })
    .flat_map(|target_table| {
        DEP_KEYS.iter().filter_map(move |key| match target_table.get(*key) {
            Some(toml::Value::Table(t)) => Some(t),
            _ => None,
        })
    });

    top_level.chain(per_target)
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
