//! The default **ranked** engine's programmatic API. Most users want the `#[decycle]` attribute;
//! these entry points are for macro authors and tooling built on decycle.

pub mod finalize;
pub(crate) mod helper;
pub(crate) mod contract;
mod nesting;
pub(crate) mod peel;
pub(crate) mod sharing;
pub(crate) mod process_module;
mod process_trait;

pub use process_module::{process_module, process_module_with_graph};
pub use process_trait::process_trait;
