//! The default **ranked** engine's programmatic API. Most users want the `#[decycle]` attribute;
//! these entry points are for macro authors and tooling built on decycle.

pub mod finalize;
pub(crate) mod helper;
mod nesting;
mod opt_out;
pub(crate) mod peel;
pub(crate) mod process_module;

pub use finalize::{check_trait, check_trait_structural};
pub use process_module::{process_module, process_module_at};
