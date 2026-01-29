#[doc(hidden)]
pub use decycle_macro::__finalize;

/// doc
pub use decycle_macro::decycle;

#[doc(hidden)]
pub trait Repeater<const RANDOM: u64, const IX: usize, PARAM: ?Sized> {
    type Type: ?Sized;
}
