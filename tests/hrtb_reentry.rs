//! C1 acceptance test: a REAL `#[decycle]` module carrying a **concrete, in-module** HRTB
//! cyclic where-bound — `for<'a> Wrap<&'a A>: Cb` — where `Wrap` is a plain generic struct
//! defined in this module (NOT a projection/`Fill`; per the C1 spec, C1 alone covers exactly
//! this concrete-in-module HRTB shape — a projection/`Fill` target additionally needs C2).
//!
//! Pre-C1, `cyclic_where_bounds` silently dropped the predicate-level `for<'a>` binder, so the
//! register-once fn's spliced `register::<Mk<Wrap<&'a A>>>(fp, Re::<Wrap<&'a A>> as usize)`
//! statement referenced an undeclared lifetime `'a` — a clean E0261 leaking out of the
//! generated code (see `docs/decycle-integration/impl-decycle-registration-and-bridge.md`,
//! C1's "the exact defect"). Post-C1 the binder is fresh-renamed to `'__dcl_hr_N` and declared
//! as a generic of the register-once fn, so this compiles, and — because the fn is called with
//! no explicit lifetime argument (only `call_targs`, which never includes a lifetime) — the
//! elided call lets region inference fill it in, which is sound because the registry key
//! (`type_name::<Mk<...>>()`, `fp`) is lifetime-erased.
//!
//! The test also proves the fix is not just cosmetic: the cyclic pair re-enters unbounded well
//! past the default `recurse_level` (10) via the runtime re-entry registry.

use decycle::decycle;

#[decycle]
mod hrtb_concrete {
    #[decycle]
    pub trait Ca {
        fn ca(&self, n: usize) -> usize;
    }
    #[decycle]
    pub trait Cb {
        fn cb(&self, n: usize) -> usize;
    }

    pub struct A;
    pub struct Wrap<T>(pub T);

    impl Ca for A
    where
        for<'a> Wrap<&'a A>: Cb,
    {
        fn ca(&self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                Wrap(&A).cb(n - 1) + 1
            }
        }
    }

    // A generic Final impl over the binder lifetime, matching what the register-once fn's
    // elided-lifetime `Re::<Wrap<&'__dcl_hr_0 A>> as usize` cast needs: `Wrap<&'a A>: Cb` holds
    // for every `'a`, not just one concrete instantiation.
    impl<'a> Cb for Wrap<&'a A>
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
fn hrtb_binder_target_compiles_and_runs_unbounded() {
    use hrtb_concrete::Ca;
    // recurse_level defaults to 10; 2000 exercises the register-once re-entry far past the
    // fixed compile-time floor -- this would panic (bounded mode's documented floor message)
    // or, pre-C1, fail to compile at all (E0261).
    assert_eq!(hrtb_concrete::A.ca(2000), 2000);
}
