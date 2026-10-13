//! `alter_macro_name` was removed in 0.6 in favour of the stand-in form `mock = path`.
#[decycle::decycle(alter_macro_name = Renamed)]
pub trait T {
    fn f(&self);
}

fn main() {}
