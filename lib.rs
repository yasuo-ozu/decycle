#![doc = include_str!("README.md")]

#[doc(hidden)]
pub use decycle_macro::__finalize;

/// Low-level helper for macro crates that want to wrap `#[decycle]` on modules.
///
/// This re-export exists for bridging: a macro crate can provide its own attribute/derive
/// macros, while still delegating `#[decycle]`-style module processing to decycle.
/// For example, a library might generate trait impls via a custom macro, but still
/// want the enclosing module to be processed by decycle to break trait cycles.
pub use decycle_impl::process_module;
/// Attribute macro that expands a module or trait to break circular trait
/// obligations within the annotated module. Also see module-level documentation.
///
/// ```rust
/// # use decycle::decycle;
/// // This annotation is required to be used within #[decycle] module
/// #[decycle]
/// trait A {
///     fn a(&self) -> ::core::primitive::usize;
/// }
///
/// #[decycle]
/// mod cycle {
///     // Trait defined out of the module
///     #[decycle]
///     use super::A;
///
///     // Direct definition
///     #[decycle]
///     trait B {
///         fn b(&self) -> usize;
///     }
///
///     struct Left(usize);
///     struct Right(usize);
///
///     impl A for Left
///     where
///         Right: B,
///     {
///         fn a(&self) -> usize {
///             self.0 + 1
///         }
///     }
///
///     impl B for Right
///     where
///         Left: A,
///     {
///         fn b(&self) -> usize {
///             self.0 + 1
///         }
///     }
/// }
/// # fn main() {}
/// ```
///
/// ## Attribute Arguments
///
/// - **Module**:
///   - `#[decycle::decycle(recurse_level = N, support_infinite_cycle = true|false, decycle = path)]`
///   - `recurse_level`: expansion depth (default 10)
///   - `support_infinite_cycle`: enables/disable infinite cycle handling (default true)
///   - `decycle`: override the path used to refer to this crate
/// - **Trait** (defined out of `#[decycle]` module):
///   - `#[decycle::decycle(marker = path, decycle = path)]`
///   - `marker`: marker type used for internal references. Required when the
///     trait definition contains non-absolute type paths so decycle can intern
///     them into a stable, globally reachable form.
///   - `decycle`: override the path used to refer to this crate
///
///
/// ### Recursion limits
/// `recurse_level` limits how many expansion stages are used to break the cycle.
/// When the limit is reached:
/// - `support_infinite_cycle = true` switches to a runtime indirection that
///   caches function pointers to allow deeper cycles.
/// - `support_infinite_cycle = false` stops with `unimplemented!` in the
///   generated code.
/// - `support_infinite_cycle = false` removes runtime shim and it makes zero-cost abstraction
///   instead of the restriction of recursion limit.
///
/// ## Example with markers
/// Use `marker` when the trait contains non-absolute paths (e.g. `super::Type`,
/// `crate::Type`, or local aliases) so decycle can intern those references.
/// The path given with `marker = <path>` argument should be practically absolute and accessible from anywhere
/// where the defined trait is used.
///
/// ```rust
/// #[decycle::decycle(marker = Marker)]
/// trait MyTrait {
///     fn value(&self) -> i32;
/// }
/// struct Marker;
/// ```
pub use decycle_macro::decycle;

/// Low-level helper for macro crates that want to wrap `#[decycle]` on traits.
///
/// This is useful when another macro crate defines or derives traits, and those traits
/// should also be valid targets for `#[decycle]`. The wrapper macro can call into this
/// function to apply decycle's transformation while keeping its own macro API.
pub use decycle_impl::process_trait;

pub use decycle_impl::finalize;

/// Internal helper used by generated code to track staged type expansion.
#[doc(hidden)]
pub trait Repeater<const RANDOM: u64, const IX: usize, PARAM: ?Sized> {
    /// The resolved type at the given stage.
    type Type: ?Sized;
}

#[doc(hidden)]
pub use decycle_impl::{proc_macro_error, type_leak};
