//! The shape that actually occurred in the wild (syan's `Parse` trait, before it was fixed), and the
//! reason `growth.rs` carries a taint fixpoint rather than only checking the parameter binding:
//!
//! * the by-value generic is spelled **`impl Trait`** in argument position, not `<S: Trait>`;
//! * the reborrowed binding is a **local derived from** the parameter (`let mut s = src.into_src();`),
//!   not the parameter itself.
//!
//! Both must still be caught, because both grow identically: `parse::<S>` → `parse::<&mut S::Out>` →
//! `parse::<&mut &mut S::Out>` → … The real crate hit this at `recursion_limit = 76` with type names
//! up to 5 129 characters.
//!
//! This is not a hypothetical: it is syan's `#[derive(Parse)]` output verbatim, before commit
//! `1e7255c` changed the signature to `parse_stream<S: ParseStream>(&mut S)`.
//!
//! ```ignore
//! fn parse(__syan_stream: impl IntoParseStream<Atom = A>) -> Result<Self, E> {
//!     let mut __syan_stream = __syan_stream.into_parse_stream();  // taint through a method call
//!     ..
//!     <FieldTy as Parse<A>>::parse(&mut __syan_stream)            // &mut at the APIT position
//! }
//! ```
//!
//! Note what is NOT flagged, and correctly so: syan worked around the growth by having `#[recurse]`
//! rewrite that argument to `erase(&mut __syan_stream)`, pinning the callee to one `&mut dyn` layer.
//! A call expression is not a place expression, so `root_ident` declines it — and it should, because
//! the erasure genuinely does close the instantiation set. The check fires on the defect, not on the
//! (since-removed) fix for it.
use decycle::decycle;

pub trait Src {
    type Out: Src;
    fn into_src(self) -> Self::Out;
    fn tick(&mut self) -> u32;
}
impl Src for u32 {
    type Out = u32;
    fn into_src(self) -> u32 {
        self
    }
    fn tick(&mut self) -> u32 {
        *self
    }
}
impl<T: Src + ?Sized> Src for &mut T {
    type Out = Self;
    fn into_src(self) -> Self {
        self
    }
    fn tick(&mut self) -> u32 {
        (**self).tick()
    }
}

#[decycle]
mod m {
    use super::Src;

    #[decycle]
    pub trait Parse {
        fn parse(src: impl Src, depth: u32) -> u32;
    }

    pub struct Expr;
    pub struct Stmt;

    impl Parse for Expr
    where
        Stmt: Parse,
    {
        fn parse(src: impl Src, depth: u32) -> u32 {
            let mut s = src.into_src();
            if depth == 0 {
                return s.tick();
            }
            <Stmt as Parse>::parse(&mut s, depth - 1)
        }
    }

    impl Parse for Stmt
    where
        Expr: Parse,
    {
        fn parse(src: impl Src, depth: u32) -> u32 {
            let mut s = src.into_src();
            if depth == 0 {
                return s.tick();
            }
            <Expr as Parse>::parse(&mut s, depth - 1)
        }
    }
}

fn main() {}
