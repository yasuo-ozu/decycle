//! Behavior tests for the **implemented** unbounded-reentry port (`docs/unbounded-reentry-plan.md`,
//! plan §4): full-height re-entry fns, `type_name`-string keys, copy-out storage, idempotent
//! bound-driven registration. Every test here (except the shallow regression guard) crashed,
//! panicked, or failed to compile on the released v0.3.0 shim — each section header names the
//! defect (D0–D7) it regresses. In particular: any width at any `recurse_level >= 1`, generic
//! methods keyed per instantiation past the floor, phantom method generics (D6), and elided
//! ref-returning methods (D7).

// The cycle's non-entry traits are only called through their ranked variants after expansion.
#![allow(dead_code)]

use decycle::decycle;

// ---------------------------------------------------------------------------------------------
// D0: any floor crossing at all. These are the crate's headline feature at default settings.
// ---------------------------------------------------------------------------------------------

#[decycle]
mod mutual_default {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn deep_recursion_default_level() {
    use mutual_default::Ca;
    assert_eq!(mutual_default::A.ca(20000), 20000);
}

#[decycle(recurse_level = 3)]
mod mutual_l3 {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn deep_mutual_level3() {
    use mutual_l3::Ca;
    assert_eq!(mutual_l3::A.ca(1000), 1000);
}

// ---------------------------------------------------------------------------------------------
// D1: registration is mid-descent, one-rank-below, set-once — a k-trait cycle needs
// recurse_level >= k+1 even once D0 is fixed. The plan's full-height re-entry + bound-driven
// idempotent registration makes any width work at any recurse_level >= 1.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 1)]
mod self_l1 {
    #[decycle]
    pub trait Count {
        fn count(&self, n: usize) -> usize;
    }
    pub struct A;
    impl Count for A
    where
        A: Count,
    {
        fn count(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.count(n - 1) + 1
            }
        }
    }
}

#[test]
fn recurse_level_1_self_cycle_unbounded() {
    use self_l1::Count;
    assert_eq!(self_l1::A.count(100), 100);
}

#[decycle(recurse_level = 1)]
mod mutual_l1 {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn recurse_level_1_mutual_cycle_unbounded() {
    use mutual_l1::Ca;
    assert_eq!(mutual_l1::A.ca(100), 100);
}

#[decycle(recurse_level = 3)]
mod wide3_l3 {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cc {
        fn cc(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    pub struct C;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        C: Cc,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                C.cc(n - 1) + 1
            }
        }
    }
    impl Cc for C
    where
        A: Ca,
    {
        fn cc(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn wide_cycle_at_level_below_width() {
    use wide3_l3::Ca;
    assert_eq!(wide3_l3::A.ca(300), 300);
}

// README's headline claim is "for any cycle width, at any `recurse_level >= 1`" — the widest
// margin between width and level (`wide3_l3` above pairs width 3 with level 3): a width-3
// cycle at the SHALLOWEST possible level, driven well past the floor.
#[decycle(recurse_level = 1)]
mod wide3_l1 {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cc {
        fn cc(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    pub struct C;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        C: Cc,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                C.cc(n - 1) + 1
            }
        }
    }
    impl Cc for C
    where
        A: Ca,
    {
        fn cc(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn wide_cycle_at_level_1_unbounded() {
    use wide3_l1::Ca;
    assert_eq!(wide3_l1::A.ca(300), 300);
}

// ---------------------------------------------------------------------------------------------
// D2: the released cell is keyed per (type, method index) and set-once, ignoring the method's
// generic instantiation — past the floor, the first instantiation serves every other one. The
// plan keys the registry per instantiation (`type_name` of a marker carrying the method
// generics). Argument-inferable generics only (the phantom form is D6 — doesn't compile today).
// ---------------------------------------------------------------------------------------------

pub trait Name: Copy {
    const NAME: &'static str;
}
#[derive(Clone, Copy)]
pub struct X;
impl Name for X {
    const NAME: &'static str = "X";
}
#[derive(Clone, Copy)]
pub struct Y;
impl Name for Y {
    const NAME: &'static str = "Y";
}

#[decycle(recurse_level = 3)]
mod generic_m {
    #[decycle]
    pub trait DeepName {
        fn deep_name<M: crate::Name>(&self, marker: M, n: usize) -> &'static str;
    }
    pub struct P;
    pub struct Q;
    impl DeepName for P
    where
        Q: DeepName,
    {
        fn deep_name<M: crate::Name>(&self, marker: M, n: usize) -> &'static str {
            if n == 0 {
                M::NAME
            } else {
                Q.deep_name(marker, n - 1)
            }
        }
    }
    impl DeepName for Q
    where
        P: DeepName,
    {
        fn deep_name<M: crate::Name>(&self, marker: M, n: usize) -> &'static str {
            if n == 0 {
                M::NAME
            } else {
                P.deep_name(marker, n - 1)
            }
        }
    }
}

#[test]
fn generic_method_multi_instantiation_past_floor() {
    use generic_m::DeepName;
    assert_eq!(generic_m::P.deep_name(X, 50), "X");
    assert_eq!(generic_m::P.deep_name(Y, 50), "Y");
}

#[decycle(recurse_level = 3)]
mod fold_m {
    #[decycle]
    pub trait Fold {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Fold for A
    where
        B: Fold,
    {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize {
            if n == 0 {
                f(0)
            } else {
                B.fold(f, n - 1) + 1
            }
        }
    }
    impl Fold for B
    where
        A: Fold,
    {
        fn fold(&self, f: impl Fn(usize) -> usize, n: usize) -> usize {
            if n == 0 {
                f(0)
            } else {
                A.fold(f, n - 1) + 1
            }
        }
    }
}

#[test]
fn impl_trait_arg_multi_closure_past_floor() {
    use fold_m::Fold;
    assert_eq!(fold_m::A.fold(|v| v + 7, 25), 25 + 7);
    assert_eq!(fold_m::A.fold(|v| v + 1000, 25), 25 + 1000);
}

// ---------------------------------------------------------------------------------------------
// D3: the released shim hands out `&'static OnceLock` transmuted from Mutex<HashMap> entries —
// concurrent first-touches can rehash the map under a live reference (latent UB). The plan's
// registry copies the `usize` out under the lock, so concurrent deep recursion is sound.
// ---------------------------------------------------------------------------------------------

#[test]
fn concurrent_deep_recursion_sound() {
    use generic_m::DeepName;
    use mutual_l3::Ca;
    let handles: Vec<_> = (0..4)
        .map(|i| {
            std::thread::spawn(move || {
                for _ in 0..50 {
                    assert_eq!(mutual_l3::A.ca(500), 500);
                    let n = if i % 2 == 0 {
                        generic_m::P.deep_name(X, 40)
                    } else {
                        generic_m::P.deep_name(Y, 40)
                    };
                    assert_eq!(n, if i % 2 == 0 { "X" } else { "Y" });
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

// ---------------------------------------------------------------------------------------------
// Regression guard — passes on the RELEASED shim and must keep passing after the port:
// recursion strictly shallower than recurse_level never reaches the floor.
// ---------------------------------------------------------------------------------------------

#[test]
fn shallow_within_level_regression() {
    use mutual_default::Ca as _;
    use mutual_l3::Ca as _;
    assert_eq!(mutual_l3::A.ca(2), 2);
    assert_eq!(mutual_default::A.ca(5), 5);
}

// ---------------------------------------------------------------------------------------------
// D6: a method generic appearing ONLY in a bound (phantom) — the released delegating impl
// re-called the ranked method without a turbofish, so this failed E0283 at compile time.
// Note: a phantom generic's floor is per-instantiation, so it needs recurse_level >= cycle
// width (here 3 >= 2) for the instantiation's own frames to register before its floor.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 3)]
mod phantom_m {
    #[decycle]
    pub trait PhantomName {
        fn phantom_name<M: crate::Name>(&self, n: usize) -> &'static str;
    }
    pub struct P;
    pub struct Q;
    impl PhantomName for P
    where
        Q: PhantomName,
    {
        fn phantom_name<M: crate::Name>(&self, n: usize) -> &'static str {
            if n == 0 {
                M::NAME
            } else {
                Q.phantom_name::<M>(n - 1)
            }
        }
    }
    impl PhantomName for Q
    where
        P: PhantomName,
    {
        fn phantom_name<M: crate::Name>(&self, n: usize) -> &'static str {
            if n == 0 {
                M::NAME
            } else {
                P.phantom_name::<M>(n - 1)
            }
        }
    }
}

#[test]
fn phantom_method_generic_past_floor() {
    use phantom_m::PhantomName;
    assert_eq!(phantom_m::P.phantom_name::<X>(50), "X");
    assert_eq!(phantom_m::P.phantom_name::<Y>(50), "Y");
}

// ---------------------------------------------------------------------------------------------
// D7: an elided ref-returning method whose output elision resolves through the receiver —
// the released floor built `fn(&Self, &str) -> &str` (fn-pointer types have no `self`
// elision rule), failing E0106 at compile time. The port names the receiver lifetime and
// binds the elided output to it.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 3)]
mod elided_m {
    #[decycle]
    pub trait PickA {
        fn pick2(&self, s: &str, n: usize) -> &str;
    }
    #[decycle]
    pub trait PickB {
        fn pick2(&self, s: &str, n: usize) -> &str;
    }
    pub struct A;
    pub struct B;
    pub static SA: A = A;
    pub static SB: B = B;
    impl PickA for A
    where
        B: PickB,
    {
        fn pick2(&self, s: &str, n: usize) -> &str {
            if n == 0 {
                "floor-a"
            } else {
                SB.pick2(s, n - 1)
            }
        }
    }
    impl PickB for B
    where
        A: PickA,
    {
        fn pick2(&self, s: &str, n: usize) -> &str {
            if n == 0 {
                "floor-b"
            } else {
                SA.pick2(s, n - 1)
            }
        }
    }
}

#[test]
fn elided_ref_return_past_floor() {
    use elided_m::PickA;
    // depth 50 (even): ends on PickA's floor-a; depth 51 (odd): PickB's floor-b.
    assert_eq!(elided_m::SA.pick2("unused", 50), "floor-a");
    assert_eq!(elided_m::SA.pick2("unused", 51), "floor-b");
}

fn panic_msg(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Residual isolation: the documented not-registered panic (a generic method's first-descent
// floor with no prior same-instantiation frame) must NOT poison the registry — the map is
// thread-local and the lookup releases its borrow before panicking, so every other cycle keeps
// working on the same thread and on fresh threads.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 1)]
mod residual_trigger {
    #[decycle]
    pub trait Gn {
        fn gn<M: crate::Name>(&self, m: M, n: usize) -> &'static str;
    }
    pub struct P;
    pub struct Q;
    impl Gn for P
    where
        Q: Gn,
    {
        fn gn<M: crate::Name>(&self, m: M, n: usize) -> &'static str {
            if n == 0 {
                M::NAME
            } else {
                Q.gn(m, n - 1)
            }
        }
    }
    impl Gn for Q
    where
        P: Gn,
    {
        fn gn<M: crate::Name>(&self, m: M, n: usize) -> &'static str {
            if n == 0 {
                M::NAME
            } else {
                P.gn(m, n - 1)
            }
        }
    }
}

#[test]
fn residual_panic_is_isolated() {
    use mutual_l1::Ca;
    use residual_trigger::Gn;
    assert_eq!(mutual_l1::A.ca(500), 500);
    let e = std::panic::catch_unwind(|| residual_trigger::P.gn(X, 5)).unwrap_err();
    assert!(
        panic_msg(e).contains("re-entry fn not registered"),
        "expected the actionable not-registered panic"
    );
    assert_eq!(mutual_l1::A.ca(500), 500);
    let fresh = std::thread::spawn(|| {
        use mutual_l1::Ca;
        mutual_l1::A.ca(500)
    })
    .join()
    .unwrap();
    assert_eq!(fresh, 500);
}

// ---------------------------------------------------------------------------------------------
// Colliding `type_name`, different layouts: closures declared in one fn all stringify as the
// same `{{closure}}`, so as desugared `impl Trait` method generics they would share a marker
// key — the layout fingerprint in the registry key keeps a 1-byte and a 32-byte closure on
// distinct entries, across threads and within one thread's nested descents.
// ---------------------------------------------------------------------------------------------

#[test]
fn colliding_closure_names_distinct_layouts() {
    use fold_m::Fold;
    let small: u8 = 7;
    let c_small = move |v: usize| v + small as usize;
    let big: [usize; 4] = [100, 200, 300, 400];
    let c_big = move |v: usize| v + big.iter().sum::<usize>();
    assert_eq!(
        std::any::type_name_of_val(&c_small),
        std::any::type_name_of_val(&c_big)
    );
    assert_ne!(std::mem::size_of_val(&c_small), std::mem::size_of_val(&c_big));

    let t1 = std::thread::spawn(move || {
        for _ in 0..2000 {
            assert_eq!(fold_m::A.fold(c_small, 40), 40 + 7);
        }
    });
    let t2 = std::thread::spawn(move || {
        for _ in 0..2000 {
            assert_eq!(fold_m::A.fold(c_big, 40), 40 + 1000);
        }
    });
    t1.join().unwrap();
    t2.join().unwrap();

    // Same-thread interleave, both alternating and genuinely nested (c_nest drives c_big's
    // whole descent from inside its own floor frame; 40-byte capture, distinct from both).
    let pad: u64 = 0;
    let c_nest = move |v: usize| v + pad as usize + fold_m::A.fold(c_big, 40);
    assert_ne!(std::mem::size_of_val(&c_nest), std::mem::size_of_val(&c_big));
    for _ in 0..100 {
        assert_eq!(fold_m::A.fold(c_small, 40), 40 + 7);
        assert_eq!(fold_m::A.fold(c_big, 40), 40 + 1000);
        assert_eq!(fold_m::A.fold(c_nest, 40), 40 + (40 + 1000));
    }
}

// ---------------------------------------------------------------------------------------------
// Rank-eater: self-recursion consumes ranks before the first generic cross-edge call, so that
// call's floor can be reached with no same-instantiation frame having run even at cycle width
// <= recurse_level — the clean actionable panic, isolated, and healed by any prior descent
// that does run a frame at the instantiation.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 3)]
mod rankeater {
    #[decycle]
    pub trait TrA {
        fn a(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait TrB {
        fn bg<M: Default>(&self, n: usize) -> usize;
    }
    pub struct A0;
    pub struct B0;
    impl TrA for A0
    where
        A0: TrA,
        B0: TrB,
    {
        fn a(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else if n.is_multiple_of(3) {
                B0.bg::<u8>(n - 1) + 1
            } else {
                A0.a(n - 1) + 1
            }
        }
    }
    impl TrB for B0
    where
        A0: TrA,
    {
        fn bg<M: Default>(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A0.a(n - 1) + 1
            }
        }
    }
}

#[test]
fn rank_eater_panics_clean_and_isolated() {
    use mutual_l3::Ca;
    use rankeater::TrA;
    // a(5)@r3 -> a(4)@r2 -> a(3)@r1 -> bg::<u8>(2) at the floor, no bg frame ran yet.
    let e = std::panic::catch_unwind(|| rankeater::A0.a(5)).unwrap_err();
    assert!(
        panic_msg(e).contains("re-entry fn not registered"),
        "expected the actionable not-registered panic"
    );
    // No poison: the same module keeps working, a good input heals the bad one, and an
    // unrelated cycle is unaffected.
    assert_eq!(rankeater::A0.a(4), 4);
    assert_eq!(rankeater::A0.a(5), 5);
    assert_eq!(rankeater::A0.a(3000), 3000);
    assert_eq!(mutual_l3::A.ca(500), 500);
}

// ---------------------------------------------------------------------------------------------
// Bare-type-param cyclic bound (impl<T: Cb> Ca for Wrap<T>): rule 1's `Self: Ca` obligation
// is undischargeable inside the rank-rewritten frame, so registration is skipped there and
// the source must still COMPILE in unbounded mode (it compiles in bounded mode). Calls that
// stay off the floor work; a floor crossing is the clean isolated panic.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 1)]
mod bareparam {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct Wrap<T>(pub T);
    pub struct Leaf;

    impl<T: Cb> Ca for Wrap<T> {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                self.0.cb(n - 1) + 1
            }
        }
    }
    impl<T: Ca> Cb for Wrap<T> {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                self.0.ca(n - 1) + 1
            }
        }
    }
    impl Ca for Leaf {
        fn ca(&self, _n: usize) -> usize {
            0
        }
    }
    impl Cb for Leaf {
        fn cb(&self, _n: usize) -> usize {
            0
        }
    }
}

#[test]
fn bare_param_bound_compiles_and_fails_closed() {
    use bareparam::Ca;
    use mutual_l1::Ca as _;
    let v = bareparam::Wrap(bareparam::Wrap(bareparam::Leaf));
    assert_eq!(v.ca(0), 0);
    let e = std::panic::catch_unwind(|| {
        use bareparam::Ca;
        bareparam::Wrap(bareparam::Wrap(bareparam::Leaf)).ca(50)
    })
    .unwrap_err();
    assert!(
        panic_msg(e).contains("re-entry fn not registered"),
        "expected the actionable not-registered panic"
    );
    assert_eq!(mutual_l1::A.ca(500), 500);
}

// ---------------------------------------------------------------------------------------------
// Review regressions (regression-diff lens): the re-entry fn must mirror the trait method's
// `unsafety` and `abi` — a safe plain-Rust `__Re_*` registered for an `unsafe fn` method was
// E0133, and for an `extern "C"` method an ABI-mismatched floor transmute (UB).
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 1)]
mod unsafe_abi_m {
    #[decycle]
    pub trait Ua {
        /// # Safety
        /// No requirements; unsafe only to exercise the emission path.
        unsafe fn ua(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Ub {
        extern "C" fn ub(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ua for A
    where
        B: Ub,
    {
        unsafe fn ua(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.ub(n - 1) + 1
            }
        }
    }
    impl Ub for B
    where
        A: Ua,
    {
        extern "C" fn ub(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                let rest = unsafe { A.ua(n - 1) };
                rest + 1
            }
        }
    }
}

#[test]
fn unsafe_and_extern_c_methods_past_floor() {
    use unsafe_abi_m::Ua;
    assert_eq!(unsafe { unsafe_abi_m::A.ua(100) }, 100);
}

// ---------------------------------------------------------------------------------------------
// F-C1: heterogeneous side-bounds cycle (regression vs v0.3.0). Naming a re-entry fn's `Self:
// T` obligation resolves through the REAL, un-ranked impls — `impl<T: Clone> Ca for A<T> where
// B<T>: Cb` needs `B<T>: Cb`, whose only impl needs `T: Default` — so every registering frame
// would need the UNION of the whole cycle's non-cyclic side-bounds, which isn't in scope
// anywhere. This must now COMPILE (it did on v0.3.0 and in bounded mode); a floor crossing
// with no prior same-instantiation registration is the clean, isolated, actionable panic.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 3)]
mod hetero_side_bounds {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A<T>(pub T);
    pub struct B<T>(pub T);

    impl<T: Clone> Ca for A<T>
    where
        B<T>: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B(self.0.clone()).cb(n - 1) + 1
            }
        }
    }
    impl<T: Default> Cb for B<T>
    where
        A<T>: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A(T::default()).ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn hetero_side_bounds_shallow_works() {
    use hetero_side_bounds::{Ca, Cb};
    assert_eq!(hetero_side_bounds::A(1i32).ca(2), 2);
    assert_eq!(hetero_side_bounds::B(2i32).cb(2), 2);
}

#[test]
fn hetero_side_bounds_deep_fails_closed() {
    use hetero_side_bounds::Ca;
    let e = std::panic::catch_unwind(|| hetero_side_bounds::A(1i32).ca(50)).unwrap_err();
    assert!(
        panic_msg(e).contains("re-entry fn not registered"),
        "expected the actionable not-registered panic"
    );
    // Not poisoned: an unrelated cycle keeps working.
    use mutual_l1::Ca as _;
    assert_eq!(mutual_l1::A.ca(500), 500);
}

// `support_infinite_cycle = false`'s documented counterpart to the floor: no re-entry
// registry at all, so a real call past `recurse_level` hits the fixed-depth leaf's
// `unimplemented!("decycle: cycle limit reached")` (README) — pinned here as an exact
// panic message, not just "it panics".
#[decycle(recurse_level = 2, support_infinite_cycle = false)]
mod bounded_past_limit {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn bounded_mode_past_limit_panics_with_documented_message() {
    use bounded_past_limit::Ca;
    // Within recurse_level: no floor crossing, no panic.
    assert_eq!(bounded_past_limit::A.ca(0), 0);
    // Past recurse_level: hits the fixed-depth leaf's `unimplemented!`.
    let e = std::panic::catch_unwind(|| bounded_past_limit::A.ca(50)).unwrap_err();
    assert!(
        panic_msg(e).contains("decycle: cycle limit reached"),
        "expected the documented bounded-mode panic message"
    );
}

// ---------------------------------------------------------------------------------------------
// F-M1: an unsized TARGET type (`impl Ca for str`) — `fingerprint_expr` used to fold
// `size_of::<Self>()` unconditionally, which doesn't compile for `str`. Must compile and run
// past the floor (the marker/alias/re-entry `S` params must also tolerate `?Sized`).
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 2)]
mod unsized_target_m {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct B;
    impl Ca for str
    where
        B: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                self.len()
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        str: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                "x".ca(n - 1) + 1
            }
        }
    }
}

#[test]
fn unsized_target_past_floor() {
    use unsized_target_m::Ca;
    // `cb`'s recursive call is `"x".ca(n - 1)` (len 1), not `self` — every base case past the
    // first step is reached on "x", not the original receiver.
    assert_eq!("hello".ca(200), 200 + 1);
}

// ---------------------------------------------------------------------------------------------
// F-M2: a `?Sized` METHOD generic — the `__Mk`/`__Fp` marker/alias declared their non-`Self`
// type params ident-only (implicit `Sized`), so a `V: ?Sized` method param failed E0277 merely
// from being named in the marker, independent of any concrete instantiation.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 2)]
mod unsized_param_m {
    #[decycle]
    pub trait Pa {
        fn peek<V: ?Sized>(&self, v: &V, n: usize) -> usize;
    }
    #[decycle]
    pub trait Pb {
        fn pb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Pa for A
    where
        B: Pb,
    {
        fn peek<V: ?Sized>(&self, v: &V, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.pb(n - 1) + core::mem::size_of_val(v).min(1)
            }
        }
    }
    impl Pb for B
    where
        A: Pa,
    {
        fn pb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.peek("s", n - 1) + 1
            }
        }
    }
}

#[test]
fn unsized_method_param_past_floor() {
    use unsized_param_m::Pa;
    assert_eq!(unsized_param_m::A.peek("s", 200), 200);
    // A Sized instantiation of the same `?Sized`-bounded param also works.
    assert_eq!(unsized_param_m::A.peek(&7u32, 200), 200);
}

// ---------------------------------------------------------------------------------------------
// A #[decycle] trait carrying an associated type: the CYCLE itself recurses only through
// plain-`usize`-typed methods (the cross-edge call never touches `Self::Out`, which is the
// known-unsupported shape — a cross-edge call whose RESULT is a sibling's associated type
// can't normalize under a generic Rank, E0369/E0277). The `Self::Out`-returning method is
// separate and is only ever called from OUTSIDE the module, at a concrete, already-resolved
// `Self` — no generic Rank involved — which is unaffected and must keep working, driven past
// the floor on the plain-typed side.
// ---------------------------------------------------------------------------------------------

#[decycle(recurse_level = 2)]
mod assoc_carrying_m {
    #[decycle]
    pub trait Ca {
        type Out;
        fn make(&self) -> Self::Out;
        fn step(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }
    pub struct A;
    pub struct B;
    impl Ca for A
    where
        B: Cb,
    {
        type Out = usize;
        fn make(&self) -> Self::Out {
            1
        }
        fn step(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.cb(n - 1) + 1
            }
        }
    }
    impl Cb for B
    where
        A: Ca,
    {
        fn cb(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.step(n - 1) + 1
            }
        }
    }
}

#[test]
fn assoc_carrying_trait_plain_recursion_past_floor_and_outside_assoc_call() {
    use assoc_carrying_m::Ca;
    // Plain-typed recursion, driven well past the floor (the associated-type machinery
    // costs noticeably more stack per frame than a plain cycle, so this stays well short of
    // `deep_recursion_default_level`'s 20000 to avoid overflowing a test thread's stack).
    assert_eq!(assoc_carrying_m::A.step(2000), 2000);
    // The `Self::Out`-returning method, called from OUTSIDE the module.
    let out: usize = assoc_carrying_m::A.make();
    assert_eq!(out, 1);
}
