//! The default **ranked** engine's programmatic API. Most users want the `#[decycle]` attribute;
//! these entry points are for macro authors and tooling built on decycle.

pub mod finalize;
pub(crate) mod helper;
mod nesting;
mod peel;
mod sharing;
mod process_module;
mod process_trait;

/// The module's obligation graph. Engine-independent — re-exported here so it can be reached
/// alongside [`process_module`] without knowing it lives in a shared module.
pub use crate::analysis::{analyze_module, EdgeKind};
pub use process_module::{process_module, process_module_with_graph};
pub use process_trait::process_trait;
