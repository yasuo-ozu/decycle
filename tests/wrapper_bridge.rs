//! Calls through a wrapper whose parts are partly outside the cycle.
//!
//! `impl Loop for A where (B, crate::other::Leaf): Loop`, with the pair's impl in the module and
//! `Leaf` implementing `Loop` outside it. The ranked engine used to replace the bound by its parts
//! (`B: Loop`, `crate::other::Leaf: Loop`), so a call through the pair had no bound fixing its rank;
//! and the pair's ranked step impl demands `Leaf: LoopRanked<R>`, which a type outside the cycle
//! never had. Only the pair's floor was left: the call compiled against it and panicked on the first
//! call ("re-entry fn not registered before the floor was reached"), and once the floor had its
//! decoy twin the call was `E0283`; by the trait path it was `E0277`. The wrapper is now kept ranked
//! next to its parts, and every type outside the cycle its chain reaches gets a ranked impl that
//! forwards to the original trait (`finalize::wrapper`). Where that cannot be written, the call is
//! still `E0283` (`tests/ui/ranked_floor_by_elimination_tuple.rs`).
//!
//! Every scenario is checked against a plain-Rust reference of the same recursion, past the floor.

#![allow(dead_code)]

pub mod other {
    pub struct Leaf;
    pub struct Leaf2(pub u32);
}

const DEEP: [u32; 10] = [0, 1, 2, 3, 5, 10, 11, 12, 40, 200];

fn ra(n: u32) -> u32 {
    if n == 0 {
        0
    } else {
        rb(n - 1) + 100
    }
}
fn rb(n: u32) -> u32 {
    if n == 0 {
        0
    } else {
        ra(n - 1) + 1
    }
}

// ---- the reported shape: the pair's impl in the module, called by method and by path ----

macro_rules! pair {
    ($m:ident [$($args:tt)*]) => {
        #[decycle::decycle($($args)*)]
        pub mod $m {
            #[decycle]
            pub trait Loop {
                fn step(&self, n: u32) -> u32;
            }

            pub struct A;
            pub struct B;

            impl<X: Loop, Y: Loop> Loop for (X, Y) {
                fn step(&self, n: u32) -> u32 {
                    self.0.step(n) + self.1.step(n)
                }
            }

            impl Loop for A
            where
                (B, crate::other::Leaf): Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else if n % 2 == 0 {
                        (B, crate::other::Leaf).step(n - 1)
                    } else {
                        <(B, crate::other::Leaf) as crate::$m::Loop>::step(&(B, crate::other::Leaf), n - 1)
                    }
                }
            }

            impl Loop for B
            where
                A: Loop,
            {
                fn step(&self, n: u32) -> u32 {
                    if n == 0 {
                        0
                    } else {
                        A.step(n - 1) + 1
                    }
                }
            }
        }

        impl $m::Loop for other::Leaf {
            fn step(&self, _n: u32) -> u32 {
                100
            }
        }
    };
}

pair!(pair_ranked []);
pair!(pair_module [module = crate::pair_module]);
pair!(pair_bounded [support_infinite_cycle = false, recurse_level = 40]);
pair!(pair_structural [structural]);

#[test]
fn a_call_through_a_pair_with_a_foreign_part() {
    use pair_ranked::Loop as _;
    use pair_module::Loop as _;
    use pair_structural::Loop as _;
    // `A.step(n)` is `B.step(n - 1) + 100` and `B.step(m)` is `A.step(m - 1) + 1`.
    assert_eq!(pair_ranked::A.step(5), 302);
    assert_eq!(pair_ranked::A.step(40), 2020);
    for n in DEEP.iter().copied().chain([2000]) {
        assert_eq!(pair_ranked::A.step(n), ra(n), "ranked A({n})");
        assert_eq!(pair_module::A.step(n), ra(n), "module A({n})");
        assert_eq!(pair_structural::A.step(n), ra(n), "structural A({n})");
        assert_eq!(pair_ranked::B.step(n), rb(n), "ranked B({n})");
    }
}

#[test]
fn a_call_through_a_pair_with_a_foreign_part_bounded() {
    use pair_bounded::Loop as _;
    // Each turn of the cycle costs three ranks (A, the pair, B); within the fixed depth it is exact.
    for n in [0, 1, 2, 3, 5, 9, 12, 20] {
        assert_eq!(pair_bounded::A.step(n), ra(n), "bounded A({n})");
    }
}

// ---- nested wrappers ----

// `Vec<(B, Leaf)>`, both wrappers implemented in the module: the floors of the Vec and of the pair
// are registered by `A`'s frame, which names them.
#[decycle::decycle]
pub mod nested_inside {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n)
        }
    }

    impl<T: Loop> Loop for Vec<T> {
        fn step(&self, n: u32) -> u32 {
            self.iter().map(|x| x.step(n)).sum()
        }
    }

    impl Loop for A
    where
        Vec<(B, crate::other::Leaf)>: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                vec![(B, crate::other::Leaf)].step(n - 1)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

impl nested_inside::Loop for other::Leaf {
    fn step(&self, _n: u32) -> u32 {
        100
    }
}

// `::std::vec::Vec<(B, Leaf)>` with the Vec's impl outside the module: the pair inside it is kept
// whole (the Vec itself has no ranked impl, so it is reached by the trait path).
#[decycle::decycle]
pub mod nested_outside {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n)
        }
    }

    impl Loop for A
    where
        ::std::vec::Vec<(B, crate::other::Leaf)>: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else if n % 2 == 0 {
                <::std::vec::Vec<(B, crate::other::Leaf)> as crate::nested_outside::Loop>::step(
                    &vec![(B, crate::other::Leaf)],
                    n - 1,
                )
            } else {
                vec![(B, crate::other::Leaf)][0].step(n - 1)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

impl<T: nested_outside::Loop> nested_outside::Loop for Vec<T> {
    fn step(&self, n: u32) -> u32 {
        self.iter().map(|x| x.step(n)).sum()
    }
}

impl nested_outside::Loop for other::Leaf {
    fn step(&self, _n: u32) -> u32 {
        100
    }
}

#[test]
fn nested_wrappers() {
    use nested_inside::Loop as _;
    use nested_outside::Loop as _;
    for n in DEEP {
        assert_eq!(nested_inside::A.step(n), ra(n), "inside A({n})");
        assert_eq!(nested_outside::A.step(n), ra(n), "outside A({n})");
    }
}

// ---- `Option`/`Box` wrappers in the module, several foreign parts ----

#[decycle::decycle]
pub mod option_box {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl<X: Loop, Y: Loop, Z: Loop> Loop for (X, Y, Z) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n) + self.2.step(n)
        }
    }

    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n)
        }
    }

    impl<T: Loop> Loop for Option<T> {
        fn step(&self, n: u32) -> u32 {
            self.as_ref().map_or(7, |x| x.step(n))
        }
    }

    impl<T: Loop + ?Sized> Loop for Box<T> {
        fn step(&self, n: u32) -> u32 {
            (**self).step(n)
        }
    }

    impl Loop for A
    where
        Option<Box<(B, crate::other::Leaf, crate::other::Leaf2)>>: Loop,
        (Option<B>, u8): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                Some(Box::new((B, crate::other::Leaf, crate::other::Leaf2(3)))).step(n - 1)
                    + (None::<B>, 5u8).step(n - 1)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

impl option_box::Loop for other::Leaf {
    fn step(&self, _n: u32) -> u32 {
        100
    }
}

impl option_box::Loop for other::Leaf2 {
    fn step(&self, _n: u32) -> u32 {
        1000 + self.0
    }
}

impl option_box::Loop for u8 {
    fn step(&self, _n: u32) -> u32 {
        u32::from(*self)
    }
}

#[test]
fn option_box_and_several_foreign_parts() {
    use option_box::Loop as _;
    fn ra(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            rb(n - 1) + 1103 + 12
        }
    }
    fn rb(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            ra(n - 1) + 1
        }
    }
    for n in DEEP {
        assert_eq!(option_box::A.step(n), ra(n), "A({n})");
    }
}

// ---- a generic part outside the cycle, a generic member, and a part that is a parameter ----

#[decycle::decycle]
pub mod generic {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A<U>(pub U);
    pub struct B<U>(pub U);
    pub struct G<T>(pub T);

    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n)
        }
    }

    // `Vec<U>` gets a generic bridge (`impl<P, R> LoopRanked<R> for Vec<P> where Vec<P>: Loop`).
    impl<U: Clone> Loop for A<U>
    where
        (B<U>, Vec<U>): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                (B(self.0.clone()), vec![self.0.clone(); 2]).step(n - 1)
            }
        }
    }

    impl<U: Clone> Loop for B<U>
    where
        A<U>: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A(self.0.clone()).step(n - 1) + 1
            }
        }
    }

    // A part that is the impl's own parameter: the parts are still reached on their own.
    impl<T> Loop for G<T>
    where
        (A<u8>, T): Loop,
        T: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            A(1u8).step(n) + self.0.step(n)
        }
    }
}

impl<U> generic::Loop for Vec<U> {
    fn step(&self, _n: u32) -> u32 {
        100 * self.len() as u32
    }
}

#[test]
fn generic_parts() {
    use generic::Loop as _;
    fn ra(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            rb(n - 1) + 200
        }
    }
    fn rb(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            ra(n - 1) + 1
        }
    }
    for n in DEEP {
        assert_eq!(generic::A(1u8).step(n), ra(n), "A<u8>({n})");
        assert_eq!(generic::A("s").step(n), ra(n), "A<&str>({n})");
        assert_eq!(generic::G(generic::B(1u8)).step(n), ra(n) + rb(n), "G({n})");
    }
}

// ---- cross-trait, with trait parameters ----

#[decycle::decycle]
pub mod cross {
    #[decycle]
    pub trait Eval<T> {
        fn eval(&self, n: u32, t: &T) -> u32;
    }
    #[decycle]
    pub trait Size<T> {
        fn size(&self, n: u32, t: &T) -> u32;
    }

    pub struct A;
    pub struct B;

    impl<T, X: Size<T>, Y: Size<T>> Size<T> for (X, Y) {
        fn size(&self, n: u32, t: &T) -> u32 {
            self.0.size(n, t) + self.1.size(n, t)
        }
    }

    impl<T> Eval<T> for A
    where
        (B, crate::other::Leaf): Size<T>,
    {
        fn eval(&self, n: u32, t: &T) -> u32 {
            if n == 0 {
                0
            } else {
                (B, crate::other::Leaf).size(n - 1, t)
            }
        }
    }

    impl<T> Size<T> for B
    where
        A: Eval<T>,
    {
        fn size(&self, n: u32, t: &T) -> u32 {
            if n == 0 {
                0
            } else {
                A.eval(n - 1, t) + 1
            }
        }
    }
}

impl<T> cross::Size<T> for other::Leaf {
    fn size(&self, _n: u32, _t: &T) -> u32 {
        100
    }
}

#[test]
fn cross_trait_with_trait_parameters() {
    use cross::Eval as _;
    for n in DEEP {
        assert_eq!(cross::A.eval(n, &0u8), ra(n), "A({n})");
        assert_eq!(cross::A.eval(n, &"s"), ra(n), "A({n})");
    }
}

// ---- a ranked supertrait (the bridge brings one for the supertrait too) ----

#[decycle::decycle]
pub mod supertrait {
    #[decycle]
    pub trait Base {
        fn base(&self) -> u32;
    }
    #[decycle]
    pub trait Loop: Base {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;

    impl<X: Base, Y: Base> Base for (X, Y) {
        fn base(&self) -> u32 {
            self.0.base() + self.1.base()
        }
    }

    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n) + self.base()
        }
    }

    impl Base for A {
        fn base(&self) -> u32 {
            1
        }
    }

    impl Base for B {
        fn base(&self) -> u32 {
            2
        }
    }

    impl Loop for A
    where
        (B, crate::other::Leaf): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                (B, crate::other::Leaf).step(n - 1)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

impl supertrait::Base for other::Leaf {
    fn base(&self) -> u32 {
        10
    }
}

impl supertrait::Loop for other::Leaf {
    fn step(&self, _n: u32) -> u32 {
        100
    }
}

#[test]
fn ranked_supertrait() {
    use supertrait::Loop as _;
    fn ra(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            rb(n - 1) + 112
        }
    }
    fn rb(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            ra(n - 1) + 1
        }
    }
    for n in DEEP {
        assert_eq!(supertrait::A.step(n), ra(n), "A({n})");
    }
}

// ---- associated type and const, a defaulted method and a generic one ----

#[decycle::decycle]
pub mod items {
    #[decycle]
    pub trait Loop {
        type Out: Default + Into<u64>;
        const K: u32;
        fn step(&self, n: u32) -> u32;
        fn scaled<T: Copy + Into<u64>>(&self, n: u32, t: T) -> u64;
        fn out(&self) -> Self::Out {
            Default::default()
        }
    }

    pub struct A;
    pub struct B;

    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        type Out = u32;
        const K: u32 = 0;
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n) + Y::K
        }
        fn scaled<T: Copy + Into<u64>>(&self, n: u32, t: T) -> u64 {
            self.0.scaled(n, t) + self.1.scaled(n, t) + self.1.out().into()
        }
    }

    impl Loop for A
    where
        (B, crate::other::Leaf): Loop,
    {
        type Out = u8;
        const K: u32 = 1;
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                (B, crate::other::Leaf).step(n - 1)
            }
        }
        fn scaled<T: Copy + Into<u64>>(&self, n: u32, t: T) -> u64 {
            if n == 0 {
                0
            } else {
                (B, crate::other::Leaf).scaled(n - 1, t)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        type Out = u8;
        const K: u32 = 2;
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
        fn scaled<T: Copy + Into<u64>>(&self, n: u32, t: T) -> u64 {
            if n == 0 {
                0
            } else {
                A.scaled(n - 1, t) + t.into()
            }
        }
    }
}

impl items::Loop for other::Leaf {
    type Out = u16;
    const K: u32 = 1000;
    fn step(&self, _n: u32) -> u32 {
        100
    }
    fn scaled<T: Copy + Into<u64>>(&self, _n: u32, t: T) -> u64 {
        10 * t.into()
    }
    fn out(&self) -> u16 {
        7
    }
}

#[test]
fn associated_items_and_a_generic_method() {
    use items::Loop as _;
    fn ra(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            rb(n - 1) + 1100
        }
    }
    fn rb(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            ra(n - 1) + 1
        }
    }
    fn sa(n: u32, t: u64) -> u64 {
        if n == 0 {
            0
        } else {
            sb(n - 1, t) + 10 * t + 7
        }
    }
    fn sb(n: u32, t: u64) -> u64 {
        if n == 0 {
            0
        } else {
            sa(n - 1, t) + t
        }
    }
    for n in DEEP {
        assert_eq!(items::A.step(n), ra(n), "A({n})");
        assert_eq!(items::A.scaled(n, 3u8), sa(n, 3), "scaled A({n})");
    }
}

// ---- no bridge where one could overlap an impl (E0119) ----

// `crate::m::C` names the member `C`, and `Leaf` and `crate::other::Leaf` may name one type: none of
// them gets a bridge, and those wrappers read as their parts, as before. `(D, Vec<u8>)` is planned
// through the module's `Vec` impl and bridges only `u8`.
#[decycle::decycle]
pub mod overlap {
    use crate::other::Leaf;

    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;
    pub struct C;
    pub struct D;
    pub struct E;
    pub struct F;

    impl<X: Loop, Y: Loop> Loop for (X, Y) {
        fn step(&self, n: u32) -> u32 {
            self.0.step(n) + self.1.step(n)
        }
    }

    impl<T: Loop> Loop for Vec<T> {
        fn step(&self, n: u32) -> u32 {
            self.iter().map(|x| x.step(n)).sum()
        }
    }

    impl Loop for C {
        fn step(&self, _n: u32) -> u32 {
            7
        }
    }

    impl Loop for A
    where
        (B, crate::overlap::C): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                B.step(n - 1) + <crate::overlap::C as crate::overlap::Loop>::step(&crate::overlap::C, n)
            }
        }
    }

    impl Loop for B
    where
        A: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }

    impl Loop for D
    where
        (E, Leaf): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                E.step(n - 1) + <Leaf as crate::overlap::Loop>::step(&Leaf, n)
            }
        }
    }

    impl Loop for E
    where
        (F, crate::other::Leaf): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                F.step(n - 1)
                    + <crate::other::Leaf as crate::overlap::Loop>::step(&crate::other::Leaf, n)
            }
        }
    }

    impl Loop for F
    where
        (D, Vec<u8>): Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                (D, vec![3u8, 4u8]).step(n - 1)
            }
        }
    }
}

impl overlap::Loop for u8 {
    fn step(&self, _n: u32) -> u32 {
        u32::from(*self)
    }
}

impl overlap::Loop for other::Leaf {
    fn step(&self, _n: u32) -> u32 {
        100
    }
}

#[test]
fn no_bridge_that_could_overlap() {
    use overlap::Loop as _;
    fn ra(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            rb(n - 1) + 7
        }
    }
    fn rb(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            ra(n - 1) + 1
        }
    }
    fn rd(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            re(n - 1) + 100
        }
    }
    fn re(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            rf(n - 1) + 100
        }
    }
    fn rf(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            rd(n - 1) + 7
        }
    }
    for n in DEEP {
        assert_eq!(overlap::A.step(n), ra(n), "A({n})");
        assert_eq!(overlap::D.step(n), rd(n), "D({n})");
    }
}

// ---- a cycle through a container the module implements (`tests/nested_participant_cycle.rs`) ----

// The floor of `impl<T: Tr> Tr for Wrap<T>` used to demand the real `T: Tr`, which went through the
// delegating impls into the rank chain again: E0275 (`Wrap<Expr>: TrRanked<()>`). It now ranks it.
#[decycle::decycle]
pub mod container {
    #[decycle]
    pub trait Tr {
        fn depth(&self) -> u32;
    }

    pub struct Wrap<T>(pub T);

    pub enum Expr {
        Lit,
        Nest(Box<Stmt>),
    }
    pub enum Stmt {
        E(Box<Expr>),
    }

    impl<T: Tr> Tr for Wrap<T> {
        fn depth(&self) -> u32 {
            self.0.depth()
        }
    }

    impl Tr for Expr
    where
        Wrap<Stmt>: Tr,
        Stmt: Tr,
    {
        fn depth(&self) -> u32 {
            match self {
                Expr::Lit => 0,
                Expr::Nest(s) => s.depth() + 1,
            }
        }
    }

    impl Tr for Stmt
    where
        Wrap<Expr>: Tr,
        Expr: Tr,
    {
        fn depth(&self) -> u32 {
            match self {
                Stmt::E(e) => e.depth(),
            }
        }
    }
}

#[test]
fn a_cycle_through_a_participating_container() {
    use container::{Expr, Stmt, Tr, Wrap};
    let mut e = Expr::Lit;
    for _ in 0..300 {
        e = Expr::Nest(Box::new(Stmt::E(Box::new(e))));
    }
    assert_eq!(e.depth(), 300);
    assert_eq!(Wrap(Expr::Lit).depth(), 0);
    assert_eq!(Wrap(Stmt::E(Box::new(Expr::Lit))).depth(), 0);
}

// ---- a module-level alias or rename of a member is no foreign type ----

// `L` and `R` name the members `B` and `A`; a bridge for either would conflict with the member's own
// ranked impls (E0119). They are spelled `self::L`, `self::R` in the bounds, which nothing bridges.
#[decycle::decycle]
pub mod alias {
    #[decycle]
    pub trait Loop {
        fn step(&self, n: u32) -> u32;
    }

    pub struct A;
    pub struct B;
    pub type L = B;
    use self::A as R;

    impl<T> Loop for Option<T>
    where
        T: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            self.as_ref().map_or(0, |x| x.step(n))
        }
    }

    impl Loop for A
    where
        Option<L>: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                Some(B).step(n - 1) + 1
            }
        }
    }

    impl Loop for B
    where
        Option<R>: Loop,
    {
        fn step(&self, n: u32) -> u32 {
            if n == 0 {
                0
            } else {
                Some(A).step(n - 1) + 2
            }
        }
    }
}

#[test]
fn an_alias_of_a_member_gets_no_bridge() {
    use alias::Loop as _;
    fn ra(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            rb(n - 1) + 1
        }
    }
    fn rb(n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            ra(n - 1) + 2
        }
    }
    for n in DEEP {
        assert_eq!(alias::A.step(n), ra(n), "A({n})");
    }
}
