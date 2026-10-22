//! Unit tests of the opt-out cycle check (`opt_out_cycles`) and of its fallback (`quiet`).
//!
//! The check reads the module after peeling; the modules here are written in that shape already
//! (one predicate per bounded type), which is also the shape of a direct bound.

use super::{opt_out_cycles, quiet};
use crate::module_path::ModuleScope;
use proc_macro2::Ident;
use std::collections::HashSet;
use syn::{parse_quote, Item, ItemMod, Path};
use template_quote::{quote, ToTokens};

/// `(bounded type as written, message)` of every report, with `module = <path>`.
fn reports_at(module: ItemMod, path: Path, recurse_level: usize) -> Vec<(String, String)> {
    let scope = ModuleScope::new(&path, &module);
    let items = module.content.unwrap().1;
    let routed: HashSet<Ident> = items
        .iter()
        .filter_map(|item| match item {
            Item::Trait(t) => Some(t.ident.clone()),
            _ => None,
        })
        .collect();
    opt_out_cycles(&items, &scope, &routed, recurse_level)
        .into_iter()
        .map(|c| (c.at.to_token_stream().to_string(), c.message))
        .collect()
}

fn reports(module: ItemMod) -> Vec<(String, String)> {
    reports_at(module, parse_quote!(crate::m), 10)
}

#[test]
fn a_direct_opt_out_on_a_cycle_is_reported_with_its_chain() {
    let found = reports(parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct C;
            impl Loop for A where crate::m::C: Loop {}
            impl Loop for C where A: Loop {}
        }
    });
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, "crate :: m :: C");
    assert!(
        found[0]
            .1
            .contains("`C`'s impl of `Loop` leads back to this impl (`A` → `C` → `A`)"),
        "{}",
        found[0].1
    );
}

#[test]
fn an_opt_out_off_every_cycle_is_not_reported() {
    let found = reports(parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct B;
            pub struct C;
            impl Loop for A where B: Loop, crate::m::C: Loop {}
            impl Loop for B where A: Loop {}
            impl Loop for C {}
        }
    });
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn the_bare_spelling_is_no_opt_out() {
    let found = reports(parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct C;
            impl Loop for A where C: Loop {}
            impl Loop for C where self::A: Loop {}
        }
    });
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_chain_through_a_generic_impl_is_not_judged() {
    let found = reports(parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct C;
            pub struct Wrap<T>(T);
            impl Loop for A where crate::m::C: Loop {}
            impl Loop for C where Wrap<A>: Loop {}
            impl<T> Loop for Wrap<T> where T: Loop {}
        }
    });
    assert!(found.is_empty(), "{found:?}");
    // Two impls for one head: which one a bound reaches depends on the arguments.
    let found = reports(parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct C;
            pub struct Wrap<T>(T);
            impl Loop for A where crate::m::C: Loop {}
            impl Loop for C where Wrap<i32>: Loop {}
            impl Loop for Wrap<u8> where A: Loop {}
            impl Loop for Wrap<i32> {}
        }
    });
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_impl_that_may_not_exist_as_written_is_no_node() {
    for attr in [
        quote!(#[cfg(feature = "x")]),
        quote!(#[cfg_attr(feature = "x", some_attr)]),
        quote!(#[some_macro]),
    ] {
        let found = reports(parse_quote! {
            mod m {
                pub trait Loop {}
                pub struct A;
                pub struct C;
                impl Loop for A where crate::m::C: Loop {}
                #attr
                impl Loop for C where A: Loop {}
            }
        });
        assert!(found.is_empty(), "{attr}: {found:?}");
    }
    // A lint level or a doc comment changes nothing.
    let found = reports(parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct C;
            /// Doc.
            #[allow(dead_code)]
            impl Loop for A where crate::m::C: Loop {}
            impl Loop for C where A: Loop {}
        }
    });
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn another_module_s_type_of_the_same_name_is_not_this_module_s() {
    for path in [
        quote!(crate::other::C),
        quote!(crate::m::inner::C),
        quote!(::krate::m::C),
        quote!(super::C),
        quote!(super::other::C),
    ] {
        let found = reports(parse_quote! {
            mod m {
                pub trait Loop {}
                pub struct A;
                pub struct C;
                impl Loop for A where #path: Loop {}
                impl Loop for C where A: Loop {}
            }
        });
        assert!(found.is_empty(), "{path}: {found:?}");
    }
}

#[test]
fn a_super_path_resolves_against_the_module_path() {
    let module: ItemMod = parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct C;
            impl Loop for A where super::m::C: Loop {}
            impl Loop for C where A: Loop {}
        }
    };
    let found = reports_at(module, parse_quote!(crate::outer::m), 10);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0]
        .1
        .starts_with("decycle: `super::m::C` names `C` of this module"));
}

#[test]
fn every_opt_out_on_a_cycle_is_reported_in_source_order() {
    let found = reports(parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct B;
            pub struct C;
            pub struct D;
            impl Loop for A where crate::m::B: Loop {}
            impl Loop for B where crate::m::A: Loop {}
            impl Loop for C where crate::m::C: Loop {}
            impl Loop for D where crate::m::A: Loop {}
        }
    });
    let at: Vec<&str> = found.iter().map(|(at, _)| at.as_str()).collect();
    // `D`'s bound reaches the cycle, but nothing leads back to `D`.
    assert_eq!(
        at,
        ["crate :: m :: B", "crate :: m :: A", "crate :: m :: C"]
    );
    assert!(found[1].1.contains("(`B` → `A` → `B`)"), "{}", found[1].1);
    assert!(
        found[2]
            .1
            .contains("it bounds `C`'s own impl of `Loop` (`C` → `C`)"),
        "{}",
        found[2].1
    );
}

#[test]
fn a_chain_longer_than_recurse_level_reaches_the_floor_first() {
    let module = |chain: proc_macro2::TokenStream| -> ItemMod {
        parse_quote! {
            mod m {
                pub trait Loop {}
                pub struct A;
                pub struct C;
                pub struct D;
                pub struct E;
                impl Loop for A where crate::m::C: Loop {}
                #chain
            }
        }
    };
    // `C` (top rank 2) → `D` (1) → `A` (the floor, which keeps the premise): overflows.
    let two = quote! {
        impl Loop for C where D: Loop {}
        impl Loop for D where A: Loop {}
    };
    // `C` (2) → `D` (1) → `E` (the floor, without the cycle edge to `A`): compiles.
    let three = quote! {
        impl Loop for C where D: Loop {}
        impl Loop for D where E: Loop {}
        impl Loop for E where A: Loop {}
    };
    let found = reports_at(module(two), parse_quote!(crate::m), 2);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].1.contains("(`A` → `C` → `D` → `A`)"),
        "{}",
        found[0].1
    );
    assert!(reports_at(module(three.clone()), parse_quote!(crate::m), 2).is_empty());
    // At the default depth the same chain is short enough.
    assert_eq!(
        reports_at(module(three), parse_quote!(crate::m), 10).len(),
        1
    );
}

#[test]
fn an_opt_out_edge_re_enters_at_the_top_rank() {
    // `C` (1) → `D` (the floor) has no cycle edge left, but `D`'s opt-out premise is kept there and
    // enters `E` at the top rank again, from which `A` is one cycle edge away.
    let module: ItemMod = parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct C;
            pub struct D;
            pub struct E;
            impl Loop for A where crate::m::C: Loop {}
            impl Loop for C where D: Loop {}
            impl Loop for D where crate::m::E: Loop {}
            impl Loop for E where A: Loop {}
        }
    };
    let found = reports_at(module, parse_quote!(crate::m), 1);
    let at: Vec<&str> = found.iter().map(|(at, _)| at.as_str()).collect();
    assert_eq!(at, ["crate :: m :: C", "crate :: m :: E"], "{found:?}");
    assert!(
        found[0].1.contains("(`A` → `C` → `D` → `E` → `A`)"),
        "{}",
        found[0].1
    );
}

#[test]
fn other_traits_and_other_shapes_are_no_edges() {
    for (bound, extra) in [
        // Another trait's impl.
        (quote!(A: Other), quote!(impl Other for A {})),
        // A binder, arguments, a type with arguments.
        (quote!(for<'a> A: Loop), quote!()),
        (quote!(A: Loop<()>), quote!()),
        (quote!(Box<A>: Loop), quote!()),
    ] {
        let found = reports(parse_quote! {
            mod m {
                pub trait Loop {}
                pub trait Other {}
                pub struct A;
                pub struct C;
                impl Loop for A where crate::m::C: Loop {}
                impl Loop for C where #bound {}
                #extra
            }
        });
        assert!(found.is_empty(), "{bound}: {found:?}");
    }
}

#[test]
fn an_alias_is_not_a_type_the_check_can_name() {
    let found = reports(parse_quote! {
        mod m {
            pub trait Loop {}
            pub struct A;
            pub struct Real;
            pub type C = Real;
            impl Loop for A where crate::m::C: Loop {}
            impl Loop for C where A: Loop {}
        }
    });
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn the_fallback_drops_the_global_bounds_of_the_routed_traits() {
    let routed: HashSet<Ident> = [parse_quote!(Loop)].into_iter().collect();
    let quieted = |item: Item| quiet(item, &routed).to_token_stream().to_string();
    let expect = |item: Item| item.to_token_stream().to_string();
    assert_eq!(
        quieted(parse_quote! {
            impl Loop for A where (B, crate::m::C): Loop, crate::m::C: Loop + Clone, B: Other {}
        }),
        expect(parse_quote! { impl Loop for A where crate::m::C: Clone, B: Other {} }),
    );
    // A bound that names a generic parameter (or `Self`) is no global predicate: a body relies on
    // it, and rustc does not prove it where it is written.
    assert_eq!(
        quieted(parse_quote! {
            impl<'a, T> Loop for Wrap<'a, T> where T: Loop, Wrap<'a, u8>: Loop, Self: Loop, A: Loop {}
        }),
        expect(parse_quote! {
            impl<'a, T> Loop for Wrap<'a, T> where T: Loop, Wrap<'a, u8>: Loop, Self: Loop {}
        }),
    );
    assert_eq!(
        quieted(parse_quote! { impl Loop for A where B: crate::m::Loop {} }),
        expect(parse_quote! { impl Loop for A {} }),
    );
    let other: Item = parse_quote! { struct S where A: Loop; };
    assert_eq!(quieted(other.clone()), expect(other));
}
