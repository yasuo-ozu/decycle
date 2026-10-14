//! `mock = path` moved to `#[announce(mock = path)]` when the trait-side `#[decycle]` was removed.
pub mod real {
    pub trait Size {
        fn size(&self) -> usize;
    }
}

pub mod stand {
    #[decycle::decycle(mock = crate::real::Size)]
    pub trait Size {
        fn size(&self) -> usize;
    }
}

fn main() {}
