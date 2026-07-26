//! Generated-name helper. (The former ρ rewrite and carrier machinery are gone: the per-member
//! terminator design wraps each natural type transparently and needs no structural mirror.)

use super::*;

/// The terminator type name for a cycle member: `__MTerm`.
pub(crate) fn term_ident(member: &str) -> Ident {
    format_ident!("__{}Term", member)
}
