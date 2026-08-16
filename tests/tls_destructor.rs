//! Pins the thread-local-destructor tolerance of the re-entry registry.
//!
//! The registry is a `thread_local!`, so a decycled call made from *another* thread-local's `Drop`
//! can run after the registry has already been torn down for that thread. `LocalKey::with` panics
//! with `AccessError` in that window, and a panic escaping a TLS destructor **aborts the process**
//! — no unwinding, no test failure, just a dead test binary. Every registry access therefore uses
//! `try_with` and degrades to the ordinary fail-closed panic instead (`lib.rs`, `register` /
//! `lookup` / `Registration::drop`).
//!
//! That fix shipped with no test at all: flipping any `try_with` back to `with` went unnoticed by
//! the whole suite. This test covers it.
//!
//! Destructor ordering is the thing that makes the window reachable. Destructors run in reverse
//! order of *initialisation*, so the thread below touches `BOMB` first and makes its decycled call
//! second — leaving the registry to be destroyed first, and `BOMB::drop` to run against a dead one.
//! Ordering is not a language guarantee, so the test accepts either outcome: what it actually
//! asserts is that the destructor completed and the process survived.

use decycle::decycle;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 0 = destructor never ran, 1 = call returned, 2 = call panicked and was contained.
static OUTCOME: AtomicUsize = AtomicUsize::new(0);

/// The panic message, when the destructor's call panicked.
static MESSAGE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

#[decycle]
mod m {
    #[decycle]
    pub trait Ev {
        fn ev(&self, n: usize) -> usize;
    }

    pub struct A;
    pub struct B;

    impl Ev for A
    where
        B: Ev,
    {
        fn ev(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                B.ev(n - 1) + 1
            }
        }
    }

    impl Ev for B
    where
        A: Ev,
    {
        fn ev(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                A.ev(n - 1) + 1
            }
        }
    }
}

struct Bomb;

impl Drop for Bomb {
    fn drop(&mut self) {
        // A depth of 30 is past the default `recurse_level = 10`, so this reaches the floor and
        // performs a real registry lookup rather than staying inside the inductive ranks.
        let r = std::panic::catch_unwind(|| {
            use m::Ev;
            m::A.ev(30)
        });
        match r {
            Ok(_) => OUTCOME.store(1, Ordering::SeqCst),
            Err(e) => {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                *MESSAGE.lock().unwrap() = msg;
                OUTCOME.store(2, Ordering::SeqCst);
            }
        }
    }
}

thread_local! {
    static BOMB: Bomb = const { Bomb };
}

#[test]
fn decycled_call_from_a_tls_destructor_does_not_abort() {
    let h = std::thread::spawn(|| {
        use m::Ev;
        // Initialise BOMB first so its destructor runs *after* the registry's.
        BOMB.with(|_| {});
        // Now initialise the registry, by making an ordinary call that crosses the floor.
        assert_eq!(m::A.ev(30), 30);
    });
    h.join().expect("worker thread panicked");

    // Reaching this line at all is most of the point: had the destructor's panic escaped, the
    // process would have aborted and this binary would never report a result.
    let outcome = OUTCOME.load(Ordering::SeqCst);
    let msg = MESSAGE.lock().unwrap().clone();
    assert_ne!(outcome, 0, "the thread-local destructor never ran");

    // With the observed ordering the registry is already gone, so the call fails closed. That it
    // fails with *decycle's* message is the actual pin: `with` in place of `try_with` would
    // surface std's `AccessError` from inside the destructor instead.
    if outcome == 2 {
        assert!(
            msg.contains("decycle: re-entry fn not registered"),
            "expected decycle's fail-closed panic from a torn-down registry, got: {msg}"
        );
    }
}
