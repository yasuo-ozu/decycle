//! Test-only bridge for the C2 (`also_rank`) acceptance test.
//!
//! Constructs `decycle_impl::finalize::FinalizeArgs` PROGRAMMATICALLY with a non-empty
//! `also_rank`, exactly the way a real wrapper macro crate (e.g. syan's `#[recurse]`) would:
//! bypassing the `#[decycle]` attribute/carrier entirely (the real integration is
//! programmatic — see decycle's D1). Everything the demo needs is hardcoded here (the
//! macro takes no input) because the whole point is exercising `also_rank`'s wiring
//! end-to-end (the projection-normalize pre-pass + `foreign_impls` injection into
//! `replacing_table`), not building a general-purpose bridge.

use decycle_impl::finalize::{finalize, AlsoRank, FinalizeArgs};
use proc_macro::TokenStream;
use syn::{parse_quote, ItemImpl, ItemTrait};
use template_quote::quote;

#[proc_macro]
pub fn also_rank_projection_demo(_input: TokenStream) -> TokenStream {
    let ca_trait: ItemTrait = parse_quote! {
        pub trait Ca {
            fn ca(&self, n: usize) -> usize;
        }
    };
    let cb_trait: ItemTrait = parse_quote! {
        pub trait Cb {
            fn cb(&self, n: usize) -> usize;
        }
    };

    // The cross-edge obligation is a PROJECTION `<G as EmptyGroup>::Fill<B>` (`Fill<B> =
    // Group<B>`) — the exact shape C2 targets (mirrors syan's `<G as EmptyGroup>::Fill<Slot>`,
    // `Fill<Slot> = Group<Slot,O,C>`). `also_rank.normalize` below rewrites it to the concrete
    // `Group<B>` BEFORE ranking; without that rewrite `reachable_side_bounds_ok` can't match
    // any impl against the literal projection type and the cross-edge registration is skipped
    // (the group floor then panics on the first call past `recurse_level`, per the C2 doc's
    // defect write-up).
    let ca_impl: ItemImpl = parse_quote! {
        impl Ca for A
        where
            <G as EmptyGroup>::Fill<B>: Cb,
        {
            fn ca(&self, n: usize) -> usize {
                if n == 0 {
                    0
                } else {
                    Group(B).cb(n - 1) + 1
                }
            }
        }
    };
    let cb_impl: ItemImpl = parse_quote! {
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
    };

    // The concrete, member-shaped foreign impl `also_rank.foreign_impls` injects into
    // `replacing_table["Cb"]` — NEVER the rank-preserving `∀Slot` wrapper (see the `AlsoRank`
    // docs: that form must be emitted by the caller directly and kept out of ranking). Its own
    // `B: Cb` where-bound is the cyclic bound decycle then ranks, closing the cycle back
    // through `B` — reducing `Group<B>: …Ranked` to `B: …Ranked`.
    let group_impl: ItemImpl = parse_quote! {
        impl Cb for Group<B>
        where
            B: Cb,
        {
            fn cb(&self, n: usize) -> usize {
                self.0.cb(n)
            }
        }
    };

    let also_rank = AlsoRank {
        normalize: vec![(
            parse_quote!(<G as EmptyGroup>::Fill<B>),
            parse_quote!(Group<B>),
        )],
        foreign_impls: vec![group_impl],
    };

    let args = FinalizeArgs {
        working_list: Vec::new(),
        traits: vec![ca_trait.clone(), cb_trait.clone()],
        contents: vec![ca_impl, cb_impl],
        recurse_level: 1,
        support_infinite_cycle: true,
        renames: Vec::new(),
        also_rank: vec![also_rank],
        decycle_path: Some(parse_quote!(::decycle)),
    };

    let generated = finalize(args);
    let out = quote! {
        pub struct A;
        pub struct B;
        pub struct G;
        pub struct Group<T>(pub T);
        pub trait EmptyGroup {
            type Fill<Slot>;
        }
        impl EmptyGroup for G {
            type Fill<Slot> = Group<Slot>;
        }
        #ca_trait
        #cb_trait
        #generated
    };
    out.into()
}
