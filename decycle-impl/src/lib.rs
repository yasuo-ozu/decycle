// `HashMap`/`VisitMut` are used by the `GenericRenamer` cluster below, reached via `process_trait`.
use std::collections::HashMap;
use syn::visit_mut::VisitMut;
use syn::*;

// ===== Ranked engine (default) =====
pub mod ranked;
// Root re-exports kept for `decycle-macro` and the internal `crate::finalize` / `crate::helper`
// paths; the user-facing docs live on `ranked` (surfaced as `decycle::ranked`).
#[doc(hidden)]
pub use ranked::finalize;
pub(crate) use ranked::helper;
#[doc(hidden)]
pub use ranked::process_module;
#[doc(hidden)]
pub use ranked::process_trait;

// ===== Structural unroll (`#[decycle(structural)]`) =====
pub mod structural;
// Aliased root re-export kept for `decycle-macro` (the public name is `structural::process_module`).
#[doc(hidden)]
pub use structural::process_module as process_module_structural;

// Rendering `syn::Generics` to token form, shared by both engines.
mod generics_fmt;

/// Emitted by both engines when a `#[decycle]` module contains no trait (nor `#[decycle] use`)
/// annotated with `#[decycle]` — nothing marks a cycle participant, so there is nothing to break.
pub(crate) const NO_DECYCLE_TRAITS_MSG: &str =
    "cannot detect traits nor `use` statement annotated with #[decycle]";

pub use proc_macro_error;
pub use type_leak;

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

/// Attributes on a trait-method impl that must be replicated onto EVERY generated copy of that
/// method (the delegating shim, each ranked/inductive copy, the floor, the re-entry fn, the
/// structural `__run`), not applied just once. These are idempotent, layer-sensitive diagnostics /
/// codegen hints: `#[track_caller]` must be on every frame for `Location::caller()` to reach the real
/// call site; a `#[allow/warn/deny/forbid(...)]` on the method must cover the copy that actually
/// carries the user's body; `#[inline]`/`#[cold]`/`#[must_use]` are hints. Deliberately EXCLUDES
/// `cfg`/`cfg_attr` (needs separate handling), `derive`, `doc`, and arbitrary attribute macros —
/// replicating those would double-run them or duplicate items.
pub(crate) fn propagated_method_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|a| {
            let p = a.path();
            p.is_ident("track_caller")
                || p.is_ident("inline")
                || p.is_ident("cold")
                || p.is_ident("allow")
                || p.is_ident("warn")
                || p.is_ident("deny")
                || p.is_ident("forbid")
                || p.is_ident("must_use")
        })
        .cloned()
        .collect()
}

/// The `#[cfg(...)]` / `#[cfg_attr(...)]` attributes on an item. decycle can't *evaluate* these (a
/// proc-macro doesn't know the target/features), so instead it REPLICATES them onto every item it
/// generates from a cfg-gated source item — the generated machinery then strips together with the
/// source under rustc's post-expansion cfg pass, instead of referencing an item rustc removed.
pub(crate) fn extract_cfg_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
        .cloned()
        .collect()
}

/// Symbol-defining attributes: `#[no_mangle]`, `#[export_name]`, `#[link_section]`. They produce a
/// FIXED externally-visible symbol, so — unlike diagnostic attrs — they must land on EXACTLY ONE
/// emitted copy of a method: the callable delegating impl on the ORIGINAL type implementing the
/// ORIGINAL trait. Replicating them onto the internal ranked/terminator copies would yield duplicate
/// symbols; dropping them from the callable copy (the previous behavior) silently ignored the user's
/// intent. So each engine keeps them on the delegating/natural copy and strips them elsewhere.
pub(crate) fn is_symbol_attr(a: &Attribute) -> bool {
    let p = a.path();
    p.is_ident("no_mangle") || p.is_ident("export_name") || p.is_ident("link_section")
}

/// The symbol-defining attributes among `attrs` (see [`is_symbol_attr`]).
pub(crate) fn symbol_attrs(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs.iter().filter(|a| is_symbol_attr(a)).cloned().collect()
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
