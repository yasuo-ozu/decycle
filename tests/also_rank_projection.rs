//! C2 acceptance test — indirect/projection obligation ranking via the `also_rank` hook.
//!
//! `alsorank_bridge::also_rank_projection_demo!()` constructs `FinalizeArgs` programmatically
//! (D1-style, bypassing the `#[decycle]` attribute/carrier) with a two-member cycle whose
//! cross-edge obligation is a PROJECTION (`<G as EmptyGroup>::Fill<B>`, `Fill<B> = Group<B>`),
//! plus a non-empty `also_rank`: one `normalize` pair rewriting that projection to the concrete
//! `Group<B>`, and one `foreign_impls` entry (`impl Cb for Group<B> where B: Cb`) injected into
//! the ranked set. Without `also_rank` this cross-edge is exactly the defect C2 documents:
//! `cyclic_where_bounds` records the bound with a projection TARGET, `unify_type_pattern` can't
//! match it against any candidate impl's `self_ty`, `reachable_side_bounds_ok` fails closed, and
//! the runtime re-entry registration for that edge is skipped (a clean panic the first time a
//! call crosses the fixed `recurse_level` floor). With `also_rank`, the projection is rewritten
//! to a concrete member bound before ranking and the foreign impl gives it a full ranked chain,
//! so the whole `A -> Group<B> -> B -> A` cycle is registered and the runtime re-entry (the
//! `core::parse::vtable`-style mechanism) carries calls past the fixed engine depth
//! indefinitely — this test drives it to depth 4000, far past `recurse_level: 1`.

// `finalize`'s generated code assumes it is nested exactly one module deep (the convention
// `process_module` establishes for a real `#[decycle] mod cycle { .. }`: `shadowing_module`'s
// own `use super::super::*;` reaches back out to whatever CONTAINS `cycle`) — so this bridge's
// output needs the same one level of wrapping a real `#[decycle] mod` would provide.
mod cycle {
    alsorank_bridge::also_rank_projection_demo!();
}
use cycle::{Ca, Cb, Group, A, B};

#[test]
fn projection_cross_edge_compiles_and_runs_unbounded() {
    // Every `ca`/`cb` step adds exactly 1 to the base case (`n == 0 => 0`), so a correct,
    // fully-connected cycle returns `n` back out at any depth — not just "didn't panic".
    assert_eq!(A.ca(4000), 4000);
    assert_eq!(B.cb(4001), 4001);
    // Exercise the injected foreign impl directly too.
    assert_eq!(Group(B).cb(3000), 3000);
}
