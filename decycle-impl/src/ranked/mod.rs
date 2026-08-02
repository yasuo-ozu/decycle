//! The default **ranked** engine's programmatic API. Most users want the `#[decycle]` attribute;
//! these entry points are for macro authors and tooling built on decycle.

pub mod finalize;
pub(crate) mod helper;
mod process_module;
mod process_trait;

pub use process_module::process_module;
pub use process_trait::process_trait;
