//! Generated-name helpers. (The former ρ rewrite and carrier machinery are gone: the per-member
//! terminator design wraps each natural type transparently and needs no structural mirror.)
//!
//! Every identifier the structural engine synthesises carries a per-expansion **nonce** suffix so it
//! can never collide with a user identifier (or a user parameter literally named `__decycle_arg1`,
//! etc.). The nonce is a deterministic `u64` hash of the module tokens: identical within one
//! expansion (so cross-referenced names — the cast helper, the terminators — agree), distinct across
//! modules. It is rendered as fixed-width hex into each name.

use super::*;

/// Build the per-expansion nonce from the module tokens. Deterministic within one compilation, so
/// every generated name in this expansion shares it.
pub(crate) fn make_nonce(seed: &TokenStream) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.to_string().hash(&mut h);
    h.finish()
}

/// The terminator type name for a cycle member: `__MTerm_<nonce>`.
pub(crate) fn term_ident(member: &str, nonce: u64) -> Ident {
    format_ident!("__{}Term_{:016x}", member, nonce)
}

/// The layout-cast helper fn name: `__decycle_cast_<nonce>`.
pub(crate) fn cast_ident(nonce: u64) -> Ident {
    format_ident!("__decycle_cast_{:016x}", nonce)
}

/// The body-holding local trait name: `__DecycleBody_<nonce>`.
pub(crate) fn body_ident(nonce: u64) -> Ident {
    format_ident!("__DecycleBody_{:016x}", nonce)
}

/// The body-holding local method name: `__run_<nonce>`.
pub(crate) fn run_ident(nonce: u64) -> Ident {
    format_ident!("__run_{:016x}", nonce)
}

/// A fresh parameter ident for a normalized (destructured / `mut` / `ref`) param at index `i`.
pub(crate) fn arg_ident(i: usize, nonce: u64) -> Ident {
    format_ident!("__decycle_arg{}_{:016x}", i, nonce)
}

/// The generic placeholder type used in a forwarding assertion. It stands for "any element type that
/// implements the trait" and lives entirely inside a `const _ {}` block (so it needs no hygiene
/// nonce), where a readable name (`__DecycleElem`) makes the failing-bound error legible:
/// `the trait bound Box<__DecycleElem>: Tr is not satisfied`.
pub(crate) fn memberx_ident() -> Ident {
    format_ident!("__DecycleElem")
}

/// The compile-time size-guard helper type used by the layout cast: `__DecycleSizeGuard_<nonce>`.
pub(crate) fn sizeguard_ident(nonce: u64) -> Ident {
    format_ident!("__DecycleSizeGuard_{:016x}", nonce)
}
