//! A trait announced for `#[decycle] use` must name its types with paths that resolve in every
//! crate. `Local` is relative to the defining module, so `#[announce]` rejects it at the
//! definition (this is what 0.5's `marker = path` interned, with type-leak). Spell it `crate::Local`.
mod defs {
    pub struct Local(pub usize);

    #[decycle::announce::announce(crate = ::decycle::announce)]
    pub trait Produce {
        fn produce(&self) -> Local;
    }
}

fn main() {}
