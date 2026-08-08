//! D1 (E3 replan §1.2): the committed `__reentry` bridge surface — a hand-constructed
//! `(marker, fingerprint)` key registered through `register` is found by the exact `lookup`
//! call a generated floor performs; registration is idempotent and per-instantiation; the
//! FNV fold constants are locked.

// This test hand-simulates the reentry registry, whose values ARE function addresses stored as
// `*const ()` (exactly what the macro emits: `Re::<..> as *const ()`). Storing them as `usize`
// instead would strip provenance and make the floor's transmuted call UB, so the pointer cast
// is the mechanism under test, not an accident.
//
// `register` records what it displaced into the innermost open `scope()`; closing that scope
// restores the displaced entries. The generated prologue opens one scope per frame.

use decycle::__reentry::{fp_fold, fp_fold_word, lookup, register, scope, FP_SEED};

/// A marker ZST — the same shape `emit_reentry_items` mints
/// (`PhantomData<(*const Target, …)>`). The key is `type_name::<Mk<..>>()` STRING content +
/// the layout fingerprint, so a hand declaration works exactly like a generated one.
struct HandMk<S: ?Sized>(core::marker::PhantomData<*const S>);

struct MemberA(#[allow(dead_code)] u64);
struct MemberB(#[allow(dead_code)] u8);

fn reentry_a(n: u32) -> u32 {
    n + 1
}
fn reentry_b(n: u32) -> u32 {
    n + 2
}

/// The E3 no-targ/no-marg fp recipe: seed folded once with the target's layout — exactly
/// `fingerprint_expr(_, target, false, <no trait generics>, &[], Some(<empty>))`.
fn fp_of<T>() -> u64 {
    fp_fold(FP_SEED, core::mem::size_of::<T>(), core::mem::align_of::<T>())
}

/// Call a looked-up entry the way a generated floor does: transmute to the fn type and invoke.
///
/// Behaviour, never pointer identity. Rust does not promise a function has a stable address —
/// identical functions may be merged and one function may have several addresses — so comparing
/// `fn as *const ()` values is checking something the language does not guarantee (miri models
/// this and hands out different addresses for repeated casts). What the registry must actually
/// guarantee is that the entry calls the RIGHT function, which is what these tests assert.
fn call(p: *const (), n: u32) -> u32 {
    unsafe { core::mem::transmute::<*const (), fn(u32) -> u32>(p)(n) }
}

#[test]
fn hand_key_round_trips_through_floor_lookup() {
    register::<HandMk<MemberA>>(fp_of::<MemberA>(), reentry_a as *const ());
    register::<HandMk<MemberB>>(fp_of::<MemberB>(), reentry_b as *const ());
    // The floor's exact call shape: lookup::<Mk<Target>>(fp), transmuted and called.
    let fa = unsafe {
        core::mem::transmute::<*const (), fn(u32) -> u32>(lookup::<HandMk<MemberA>>(
            fp_of::<MemberA>(),
        ))
    };
    let fb = unsafe {
        core::mem::transmute::<*const (), fn(u32) -> u32>(lookup::<HandMk<MemberB>>(
            fp_of::<MemberB>(),
        ))
    };
    assert_eq!(fa(41), 42);
    assert_eq!(fb(40), 42);
}

#[test]
fn registration_is_idempotent_and_per_instantiation() {
    // `register_all_members` may run on every facade entry — same key, same fn, harmless:
    register::<HandMk<MemberA>>(fp_of::<MemberA>(), reentry_a as *const ());
    register::<HandMk<MemberA>>(fp_of::<MemberA>(), reentry_a as *const ());
    assert_eq!(call(lookup::<HandMk<MemberA>>(fp_of::<MemberA>()), 41), 42);
    // Distinct instantiations never collide even at IDENTICAL layout (fp equal): the marker's
    // type_name differs — the spike's P3/P7 cross-T guarantee.
    struct SameLayoutAsA(#[allow(dead_code)] u64);
    register::<HandMk<SameLayoutAsA>>(fp_of::<SameLayoutAsA>(), reentry_b as *const ());
    assert_eq!(fp_of::<MemberA>(), fp_of::<SameLayoutAsA>());
    assert_eq!(call(lookup::<HandMk<MemberA>>(fp_of::<MemberA>()), 41), 42);
    assert_eq!(call(lookup::<HandMk<SameLayoutAsA>>(fp_of::<SameLayoutAsA>()), 40), 42);
}

/// Registration is FRAME-SCOPED: an inner registration of the SAME key shadows the outer one
/// only while its guard lives, and the outer value is restored when the inner guard drops. This
/// is what stops a nested descent (a callback in the middle of a method body) from making the
/// floor call the wrong fn, and what stops a panicked descent from leaving a stale entry.
#[test]
fn registration_is_frame_scoped_and_restores_on_drop() {
    struct Solo(#[allow(dead_code)] u64);
    let fp = fp_of::<Solo>();

    let _outer = scope();
    register::<HandMk<Solo>>(fp, reentry_a as *const ());
    assert_eq!(call(lookup::<HandMk<Solo>>(fp), 41), 42);
    {
        let _inner = scope();
        register::<HandMk<Solo>>(fp, reentry_b as *const ());
        assert_eq!(call(lookup::<HandMk<Solo>>(fp), 40), 42);
    }
    // Inner scope closed => the outer frame's entry is back, not the inner one.
    assert_eq!(call(lookup::<HandMk<Solo>>(fp), 41), 42);
}

/// Only SHADOWING is scoped. A registration that created its slot deliberately outlives the
/// frame, because two documented behaviors depend on it: the bare-param case is "unbounded once
/// primed" (`tests/bareparam_reentry.rs`), and a descent that hit the fail-closed panic leaves
/// its registrations behind so a later good call succeeds (`tests/limitation.rs`).
#[test]
fn a_created_registration_outlives_its_frame() {
    struct Primed(#[allow(dead_code)] u32);
    let fp = fp_of::<Primed>();
    {
        let _g = scope();
        register::<HandMk<Primed>>(fp, reentry_a as *const ());
        assert_eq!(call(lookup::<HandMk<Primed>>(fp), 41), 42);
    }
    // Scope closed, but nothing was displaced, so the priming entry stays.
    assert_eq!(
        call(lookup::<HandMk<Primed>>(fp), 41),
        42,
        "a priming registration must survive its frame",
    );
}

#[test]
fn fnv_fold_constants_locked() {
    const FP_PRIME: u64 = 0x100000001b3;
    assert_eq!(FP_SEED, 0xcbf29ce484222325);
    let acc = (FP_SEED ^ 8).wrapping_mul(FP_PRIME);
    assert_eq!(fp_fold(FP_SEED, 8, 8), (acc ^ 8).wrapping_mul(FP_PRIME));
    assert_eq!(fp_fold_word(FP_SEED, 7), (FP_SEED ^ 7).wrapping_mul(FP_PRIME));
    assert_ne!(fp_fold(FP_SEED, 8, 8), fp_fold(FP_SEED, 16, 8));
}
