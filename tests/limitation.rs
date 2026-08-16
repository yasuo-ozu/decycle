//! Runtime (non-UI) limitation tests: the ranked engine's documented *fail-closed* boundaries.
//!
//! Some ranked cycles cannot be registered for unbounded re-entry (a generic method first reached
//! past the floor, a bare-type-param cyclic bound, a heterogeneous side-bound cycle) or are run in
//! bounded mode past their `recurse_level`. These are NOT compile errors — decycle fails CLOSED at
//! the recursion floor with an actionable panic, and (critically) that panic must stay ISOLATED: it
//! releases the thread-local registry borrow before unwinding, so every other cycle keeps working on
//! the same thread and on fresh threads. Each test here drives one such boundary and asserts both the
//! documented panic message and the no-poison property. The compile-time rejections live in
//! `tests/ui/*.rs`; the structural engine has no runtime limitation of this kind (it is compile-time
//! only), so every test here is ranked.
#![allow(dead_code)]

use decycle::decycle;

/// Extract a panic payload's message (`String` or `&str`).
fn panic_msg(e: Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// A plain working cycle used as the "unrelated cycle is unaffected" control after each fail-closed
/// panic — any healthy unbounded cycle proves the registry wasn't poisoned.
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

/// A `Copy` marker type carried as a generic method argument, to force a per-instantiation key.
pub trait Name: Copy {
    const NAME: &'static str;
}
#[derive(Clone, Copy)]
pub struct X;
impl Name for X {
    const NAME: &'static str = "X";
}

// ---------------------------------------------------------------------------------------------
// Residual isolation: the documented not-registered panic (a generic method's first-descent
// floor with no prior same-instantiation frame) must NOT poison the registry — the map is
// thread-local and the lookup releases its borrow before panicking, so every other cycle keeps
// working on the same thread and on fresh threads.
//
// `residual_trigger` below is no longer one of those floors: a cross edge to a generic method is
// now registered from the prologue of the method that CAN name its instantiation (here the
// caller's own `M`), so this cycle runs. It is kept as the positive control for that fix; the
// still-fail-closed shape — a generic method reached from a NON-generic caller, whose type
// argument is chosen inside the user's body and which therefore nothing in scope can name — is
// `rankeater` below, and it is what drives the no-poison assertions now.
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
    use rankeater::TrA;
    use residual_trigger::Gn;
    assert_eq!(mutual_l1::A.ca(500), 500);
    // A generic method reached across a cross edge from a caller that names its instantiation is
    // registered now — this used to be a fail-closed floor.
    assert_eq!(residual_trigger::P.gn(X, 5), "X");
    // The residual shape: `bg::<u8>` is reached from the NON-generic `a`, so no scope on the way
    // down can spell its type argument and its floor still fails closed.
    let e = std::panic::catch_unwind(|| rankeater::A0.a(5)).unwrap_err();
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
// Rank exhaustion: a generic sibling method (`bg`) first reached at the floor, with no prior
// same-instantiation frame, panics cleanly and stays isolated.
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
            } else if n % 3 == 0 {
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
    use mutual_l1::Ca;
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
    assert_eq!(mutual_l1::A.ca(500), 500);
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
// Heterogeneous side-bound cycle (`impl<T> Ca for A<T> where B<T>: Cb`): the registering impl's
// bounds don't cover the sibling obligation at every instantiation, so a deep crossing fails
// closed. A shallow, within-level call is exercised (positively) in `unbounded_reentry.rs`.
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

// ---------------------------------------------------------------------------------------------
// Bounded mode (`support_infinite_cycle = false`): no re-entry registry at all, so a real call
// past `recurse_level` hits the fixed-depth leaf's `unimplemented!("decycle: cycle limit
// reached")` (README) — pinned here as an exact panic message, not just "it panics".
// ---------------------------------------------------------------------------------------------

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
