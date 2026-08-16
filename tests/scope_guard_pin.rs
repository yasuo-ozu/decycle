//! Pins the generated re-entry **scope guard**, which nothing else in the suite tests.
//!
//! The ranked engine's method prologue opens a `__reentry::scope()` and binds it to a named
//! nonce ident (`decycle-impl/src/ranked/finalize.rs`, `let #scope_guard = ..`). Holding that
//! binding for the rest of the body is what makes a registration last exactly as long as the
//! descent it serves: when the guard drops, every entry the scope *displaced* is restored.
//!
//! Two comments in the tree warn about getting this wrong — the codegen site says "Binding to a
//! named nonce ident rather than `_` matters — `let _ = ..` would drop it right here", and
//! `Registration` carries a `#[must_use]` saying the same. Neither was executable: an audit
//! (2026-08-15) changed that single token to `let _ = ..` and the **entire suite stayed green**,
//! including all trybuild cases. `let _` also silences the `#[must_use]`, so clippy missed it too.
//!
//! To check this test still bites, make that one-token change and run it: it must fail.
//!
//! Why it needs *named* block-local types rather than closures: `assert_key_encodable` rejects any
//! key rendering to `{{closure}}`, so a closure-instantiated method panics before it ever reaches
//! a registration — which is exactly why the two tests in `ub_regressions.rs` that are named for
//! the registry defects no longer exercise this path at all. Two `struct S` declared in different
//! blocks of one function both render `<crate>::<fn>::S` and share a layout, so they land in one
//! registry slot without tripping that guard.

#![allow(dead_code)]

use decycle::decycle;
use std::cell::{Cell, RefCell};

pub trait Op: Copy {
    fn call(&self, v: usize) -> usize;
}

thread_local! {
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
    static HOOK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Runs the installed hook once, re-entrantly guarded. Called from the middle of a cyclic method
/// body, so the nested descent it starts happens *after* this frame's prologue has registered.
pub fn hook() {
    if IN_HOOK.with(|c| c.get()) {
        return;
    }
    IN_HOOK.with(|c| c.set(true));
    let h = HOOK.with(|s| s.borrow_mut().take());
    if let Some(h) = &h {
        h();
    }
    HOOK.with(|s| *s.borrow_mut() = h);
    IN_HOOK.with(|c| c.set(false));
}

#[decycle]
mod collide {
    #[decycle]
    pub trait Fold {
        fn fold<F: crate::Op>(&self, f: F, n: usize) -> usize;
    }

    pub struct A;

    impl Fold for A
    where
        A: Fold,
    {
        fn fold<F: crate::Op>(&self, f: F, n: usize) -> usize {
            if n == 0 {
                f.call(0)
            } else {
                // A nested descent started from user code partway through the body. Without the
                // scope guard this re-registers the *inner* instantiation into the shared slot
                // and never puts the outer one back, so the outer descent's floor calls it.
                crate::hook();
                A.fold(f, n - 1) + 1
            }
        }
    }
}

#[test]
fn nested_descent_does_not_steal_the_outer_frames_registration() {
    use collide::Fold;

    // Inner `S`: drives the nested descent from inside the outer body.
    {
        #[derive(Clone, Copy)]
        struct S(usize);
        impl Op for S {
            fn call(&self, v: usize) -> usize {
                v * 3 + 100_000
            }
        }
        let inner = S(3);
        HOOK.with(|s| {
            *s.borrow_mut() = Some(Box::new(move || {
                let _ = collide::A.fold(inner, 12);
            }))
        });
    }

    // Outer `S`: same name, same layout, hence the same registry key as the inner one.
    let got = {
        #[derive(Clone, Copy)]
        struct S(usize);
        impl Op for S {
            fn call(&self, v: usize) -> usize {
                v + 7
            }
        }
        // Both spellings really do render identically; if this ever stops being true the test has
        // stopped exercising a collision and must be rewritten rather than deleted.
        assert!(
            std::any::type_name::<S>().ends_with("::S"),
            "expected a block-local named type, got {}",
            std::any::type_name::<S>()
        );
        collide::A.fold(S(7), 30)
    };
    HOOK.with(|s| *s.borrow_mut() = None);

    // 30 recursion levels each add 1, and the floor runs the OUTER `S` (0 + 7).
    assert_eq!(
        got, 37,
        "the floor dispatched the nested descent's instantiation instead of this frame's"
    );
}
