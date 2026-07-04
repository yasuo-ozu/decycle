//! D1 (E3 replan §1.2): the committed `__reentry` bridge surface — a hand-constructed
//! `(marker, fingerprint)` key registered through `register` is found by the exact `lookup`
//! call a generated floor performs; registration is idempotent and per-instantiation; the
//! FNV fold constants are locked.

use decycle::__reentry::{fp_fold, fp_fold_word, lookup, register, FP_SEED};

/// A syan-style marker ZST — the same shape `emit_reentry_items` mints
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

#[test]
fn hand_key_round_trips_through_floor_lookup() {
    register::<HandMk<MemberA>>(fp_of::<MemberA>(), reentry_a as usize);
    register::<HandMk<MemberB>>(fp_of::<MemberB>(), reentry_b as usize);
    // The floor's exact call shape: lookup::<Mk<Target>>(fp), transmuted and called.
    let fa = unsafe {
        core::mem::transmute::<usize, fn(u32) -> u32>(lookup::<HandMk<MemberA>>(fp_of::<MemberA>()))
    };
    let fb = unsafe {
        core::mem::transmute::<usize, fn(u32) -> u32>(lookup::<HandMk<MemberB>>(fp_of::<MemberB>()))
    };
    assert_eq!(fa(41), 42);
    assert_eq!(fb(40), 42);
}

#[test]
fn registration_is_idempotent_and_per_instantiation() {
    // `register_all_members` may run on every facade entry — same key, same fn, harmless:
    register::<HandMk<MemberA>>(fp_of::<MemberA>(), reentry_a as usize);
    register::<HandMk<MemberA>>(fp_of::<MemberA>(), reentry_a as usize);
    assert_eq!(lookup::<HandMk<MemberA>>(fp_of::<MemberA>()), reentry_a as usize);
    // Distinct instantiations never collide even at IDENTICAL layout (fp equal): the marker's
    // type_name differs — the spike's P3/P7 cross-T guarantee.
    struct SameLayoutAsA(#[allow(dead_code)] u64);
    register::<HandMk<SameLayoutAsA>>(fp_of::<SameLayoutAsA>(), reentry_b as usize);
    assert_eq!(fp_of::<MemberA>(), fp_of::<SameLayoutAsA>());
    assert_eq!(lookup::<HandMk<MemberA>>(fp_of::<MemberA>()), reentry_a as usize);
    assert_eq!(
        lookup::<HandMk<SameLayoutAsA>>(fp_of::<SameLayoutAsA>()),
        reentry_b as usize
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
