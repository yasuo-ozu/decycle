//! The **existing** decycle algorithm: a `type-leak`-backed ranked engine driven by the
//! `finalize` metadata ping-pong. Selected by `#[decycle]` on a module *without* `structural`.
//!
//! (Moved here verbatim to distinguish it from the `structural` unroll — the shared helpers it
//! uses, `crate::{is_decycle_attribute, ident_to_path, get_random, randomize_impl_generics,
//! GenericRenamer}`, still live at the crate root; `crate::finalize` / `crate::helper` resolve
//! through the re-exports in `lib.rs`.)

pub mod finalize;
pub(crate) mod helper;
pub mod process_module;
#[cfg(feature = "type-leak")]
pub mod process_trait;
