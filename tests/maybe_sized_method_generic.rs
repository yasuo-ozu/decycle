//! A participating method whose OWN generic parameter is `?Sized`.
//!
//! Regression: the `__Fp_` fn-pointer alias declares each method type param with
//! `DeclBounds::Unsized` — i.e. it already emits `T: ?::core::marker::Sized` — and then copied the
//! user's bounds verbatim into the alias's `where` clause. When the user had written `?Sized`
//! themselves that produced a *second* relaxed bound on the same parameter, which rustc rejects:
//!
//! ```text
//! error[E0203]: duplicate relaxed `Sized` bounds
//!   --> #[decycle]
//!    |         fn parse_stream<Strm: Stream<Atom = Atom> + ?Sized>(s: &mut Strm) -> ...
//!    |                                                     ^^^^^^
//! ```
//!
//! The shape is not exotic: it is what a parser trait looks like when the stream is passed as
//! `&mut S` so that recursion reborrows rather than nesting (`?Sized` lets `S` also be
//! `dyn Stream`). Both spellings must work, so this file exercises the method generic with and
//! without the relaxation, and drives each unbounded past the rank floor.

use decycle::decycle;

pub trait Stream {
    type Atom;
    fn next(&mut self) -> Option<Self::Atom>;
}
impl<T: ?Sized + Stream> Stream for &'_ mut T {
    type Atom = T::Atom;
    fn next(&mut self) -> Option<Self::Atom> {
        T::next(self)
    }
}
pub struct Src(pub std::vec::IntoIter<u8>);
impl Stream for Src {
    type Atom = u8;
    fn next(&mut self) -> Option<u8> {
        self.0.next()
    }
}

/// The method generic carries `?Sized` — the case that used to fail.
#[decycle]
mod relaxed {
    use super::Stream;

    #[decycle]
    pub trait Parse<Atom>: Sized {
        type Error;
        fn parse_stream<S: Stream<Atom = Atom> + ?Sized>(s: &mut S) -> Result<Self, Self::Error>;
    }

    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl<Atom> Parse<Atom> for A
    where
        B: Parse<Atom>,
    {
        type Error = ();
        fn parse_stream<S: Stream<Atom = Atom> + ?Sized>(s: &mut S) -> Result<Self, Self::Error> {
            Ok(A(Box::new(
                <B as Parse<Atom>>::parse_stream(&mut *s).map_err(|_| ())?,
            )))
        }
    }

    impl<Atom> Parse<Atom> for B
    where
        A: Parse<Atom>,
    {
        type Error = ();
        fn parse_stream<S: Stream<Atom = Atom> + ?Sized>(s: &mut S) -> Result<Self, Self::Error> {
            match s.next() {
                Some(_) => Ok(B(Some(Box::new(
                    <A as Parse<Atom>>::parse_stream(&mut *s).map_err(|_| ())?,
                )))),
                None => Ok(B(None)),
            }
        }
    }
}

/// The same cycle with a plain `Sized` method generic, so the fix cannot regress that path.
#[decycle]
mod sized {
    use super::Stream;

    #[decycle]
    pub trait Parse<Atom>: Sized {
        type Error;
        fn parse_stream<S: Stream<Atom = Atom>>(s: &mut S) -> Result<Self, Self::Error>;
    }

    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl<Atom> Parse<Atom> for A
    where
        B: Parse<Atom>,
    {
        type Error = ();
        fn parse_stream<S: Stream<Atom = Atom>>(s: &mut S) -> Result<Self, Self::Error> {
            Ok(A(Box::new(
                <B as Parse<Atom>>::parse_stream(&mut *s).map_err(|_| ())?,
            )))
        }
    }

    impl<Atom> Parse<Atom> for B
    where
        A: Parse<Atom>,
    {
        type Error = ();
        fn parse_stream<S: Stream<Atom = Atom>>(s: &mut S) -> Result<Self, Self::Error> {
            match s.next() {
                Some(_) => Ok(B(Some(Box::new(
                    <A as Parse<Atom>>::parse_stream(&mut *s).map_err(|_| ())?,
                )))),
                None => Ok(B(None)),
            }
        }
    }
}

fn depth_relaxed(a: &relaxed::A) -> usize {
    let (mut d, mut cur) = (0usize, &*a.0);
    while let Some(inner) = &cur.0 {
        d += 1;
        cur = &*inner.0;
    }
    d
}

fn depth_sized(a: &sized::A) -> usize {
    let (mut d, mut cur) = (0usize, &*a.0);
    while let Some(inner) = &cur.0 {
        d += 1;
        cur = &*inner.0;
    }
    d
}

#[test]
fn maybe_sized_method_generic_is_unbounded() {
    use relaxed::Parse;
    let mut src = Src(vec![1u8; 2000].into_iter());
    let a = <relaxed::A as Parse<u8>>::parse_stream(&mut src).unwrap();
    assert_eq!(depth_relaxed(&a), 2000);
}

/// `?Sized` is load-bearing: the same cycle driven through a trait object.
#[test]
fn maybe_sized_method_generic_accepts_a_trait_object() {
    use relaxed::Parse;
    let mut src = Src(vec![1u8; 64].into_iter());
    let dynamic: &mut dyn Stream<Atom = u8> = &mut src;
    let a = <relaxed::A as Parse<u8>>::parse_stream(dynamic).unwrap();
    assert_eq!(depth_relaxed(&a), 64);
}

#[test]
fn sized_method_generic_still_works() {
    use sized::Parse;
    let mut src = Src(vec![1u8; 2000].into_iter());
    let a = <sized::A as Parse<u8>>::parse_stream(&mut src).unwrap();
    assert_eq!(depth_sized(&a), 2000);
}

// ── neighbouring shapes ───────────────────────────────────────────────────────────────────────
//
// Which of these actually exercise the bug was determined by reverting the fix and rerunning:
// `only_bound` and `two_generics` FAIL without it; `where_clause` does not (that spelling goes
// through `m_where`, which was never duplicated). The last is kept as a guard, labelled honestly.

/// `?Sized` as the param's ONLY bound. Exercises the branch where filtering the relaxation leaves
/// nothing, so the alias must emit no `where` predicate for that param at all rather than an empty
/// one. **Catches the bug.** `?Sized` is load-bearing here: `S` is instantiated at `[u8]`.
#[decycle]
mod only_bound {
    #[decycle]
    pub trait Walk: Sized {
        type Error;
        fn walk<S: ?Sized + AsRef<[u8]>>(s: &S, n: usize) -> Result<Self, Self::Error>;
    }

    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl Walk for A
    where
        B: Walk,
    {
        type Error = ();
        fn walk<S: ?Sized + AsRef<[u8]>>(s: &S, n: usize) -> Result<Self, Self::Error> {
            Ok(A(Box::new(<B as Walk>::walk(s, n).map_err(|_| ())?)))
        }
    }

    impl Walk for B
    where
        A: Walk,
    {
        type Error = ();
        fn walk<S: ?Sized + AsRef<[u8]>>(s: &S, n: usize) -> Result<Self, Self::Error> {
            if n == 0 {
                Ok(B(None))
            } else {
                Ok(B(Some(Box::new(
                    <A as Walk>::walk(s, n - 1).map_err(|_| ())?,
                ))))
            }
        }
    }
}

/// Two method generics, only one relaxed — the filter must not disturb the other. **Catches the bug.**
#[decycle]
mod two_generics {
    use super::Stream;

    #[decycle]
    pub trait Parse<Atom>: Sized {
        type Error;
        fn parse_stream<S: Stream<Atom = Atom> + ?Sized, T: Clone>(
            s: &mut S,
            t: T,
        ) -> Result<Self, Self::Error>;
    }

    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl<Atom> Parse<Atom> for A
    where
        B: Parse<Atom>,
    {
        type Error = ();
        fn parse_stream<S: Stream<Atom = Atom> + ?Sized, T: Clone>(
            s: &mut S,
            t: T,
        ) -> Result<Self, Self::Error> {
            Ok(A(Box::new(
                <B as Parse<Atom>>::parse_stream(&mut *s, t).map_err(|_| ())?,
            )))
        }
    }

    impl<Atom> Parse<Atom> for B
    where
        A: Parse<Atom>,
    {
        type Error = ();
        fn parse_stream<S: Stream<Atom = Atom> + ?Sized, T: Clone>(
            s: &mut S,
            t: T,
        ) -> Result<Self, Self::Error> {
            match s.next() {
                Some(_) => Ok(B(Some(Box::new(
                    <A as Parse<Atom>>::parse_stream(&mut *s, t).map_err(|_| ())?,
                )))),
                None => Ok(B(None)),
            }
        }
    }
}

/// `?Sized` spelled in a `where` clause instead of inline. This did **not** reproduce the bug —
/// method where-predicates are carried separately and were never duplicated. Kept so that a future
/// change to either path has to keep both spellings working.
#[decycle]
mod where_clause {
    use super::Stream;

    #[decycle]
    pub trait Parse<Atom>: Sized {
        type Error;
        fn parse_stream<S>(s: &mut S) -> Result<Self, Self::Error>
        where
            S: Stream<Atom = Atom> + ?Sized;
    }

    pub struct A(pub Box<B>);
    pub struct B(pub Option<Box<A>>);

    impl<Atom> Parse<Atom> for A
    where
        B: Parse<Atom>,
    {
        type Error = ();
        fn parse_stream<S>(s: &mut S) -> Result<Self, Self::Error>
        where
            S: Stream<Atom = Atom> + ?Sized,
        {
            Ok(A(Box::new(
                <B as Parse<Atom>>::parse_stream(&mut *s).map_err(|_| ())?,
            )))
        }
    }

    impl<Atom> Parse<Atom> for B
    where
        A: Parse<Atom>,
    {
        type Error = ();
        fn parse_stream<S>(s: &mut S) -> Result<Self, Self::Error>
        where
            S: Stream<Atom = Atom> + ?Sized,
        {
            match s.next() {
                Some(_) => Ok(B(Some(Box::new(
                    <A as Parse<Atom>>::parse_stream(&mut *s).map_err(|_| ())?,
                )))),
                None => Ok(B(None)),
            }
        }
    }
}

#[test]
fn relaxation_as_the_only_bound_is_unbounded() {
    use only_bound::Walk;
    let src: &[u8] = b"unsized"; // `S = [u8]`: the relaxation is doing real work
    let a = <only_bound::A as Walk>::walk(src, 2000).unwrap();
    let (mut d, mut cur) = (0usize, &*a.0);
    while let Some(inner) = &cur.0 {
        d += 1;
        cur = &*inner.0;
    }
    assert_eq!(d, 2000);
}

#[test]
fn a_second_unrelaxed_method_generic_is_undisturbed() {
    use two_generics::Parse;
    let mut src = Src(vec![1u8; 2000].into_iter());
    let a = <two_generics::A as Parse<u8>>::parse_stream(&mut src, "carried").unwrap();
    let (mut d, mut cur) = (0usize, &*a.0);
    while let Some(inner) = &cur.0 {
        d += 1;
        cur = &*inner.0;
    }
    assert_eq!(d, 2000);
}

#[test]
fn the_where_clause_spelling_also_works() {
    use where_clause::Parse;
    let mut src = Src(vec![1u8; 2000].into_iter());
    let a = <where_clause::A as Parse<u8>>::parse_stream(&mut src).unwrap();
    let (mut d, mut cur) = (0usize, &*a.0);
    while let Some(inner) = &cur.0 {
        d += 1;
        cur = &*inner.0;
    }
    assert_eq!(d, 2000);
}
