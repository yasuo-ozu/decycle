//! Unit tests of the structural engine's own passes.

use syn::parse_quote;

fn expand(module: syn::ItemMod) -> String {
    crate::structural::process_module(module, &parse_quote!(::decycle)).to_string()
}

/// The impls of a cycle (here `A`, `B`, both of trait `Tr`) share their leaf premises: each one's
/// generated impls — the terminator's and the natural type's — carry the other's, so `A`'s
/// terminator, which reaches `B` through `B`'s natural impl, can prove `T: Clone`. An impl outside
/// the cycle (`C`) and a disjoint cycle (`D`, `E`) are re-emitted, or generated, without them.
#[test]
fn cycle_members_share_their_leaf_premises() {
    let out = expand(parse_quote! {
        mod m {
            #[decycle] pub trait Tr<T> { fn f(&self, t: &T) -> u32; }
            pub struct A; pub struct B; pub struct C; pub struct D; pub struct E;
            impl<T> Tr<T> for A where B: Tr<T> { fn f(&self, t: &T) -> u32 { B.f(t) } }
            impl<T> Tr<T> for B where A: Tr<T>, T: Clone { fn f(&self, t: &T) -> u32 { 0 } }
            impl<T> Tr<T> for C where A: Tr<T> { fn f(&self, t: &T) -> u32 { 1 } }
            impl<T> Tr<T> for D where E: Tr<T>, T: Default { fn f(&self, t: &T) -> u32 { 2 } }
            impl<T> Tr<T> for E where D: Tr<T> { fn f(&self, t: &T) -> u32 { 3 } }
        }
    });
    let squished: String = out.split_whitespace().collect();
    let impls_for = |ty: &str| -> Vec<&str> {
        squished
            .split("impl<")
            .filter(|chunk| chunk.contains(&format!("for{ty}")))
            .collect()
    };
    for ty in ["A", "B"] {
        let headers = impls_for(ty);
        assert!(!headers.is_empty(), "{ty} has generated impls");
        assert!(
            headers
                .iter()
                .all(|h| h.contains("T:Clone") && !h.contains("T:Default")),
            "{ty}'s impls carry the cycle's premise, and only it: {headers:?}"
        );
    }
    let c = impls_for("C");
    assert!(
        c.iter()
            .all(|h| !h.contains("Clone") && !h.contains("Default")),
        "C is outside the cycle: {c:?}"
    );
    for ty in ["D", "E"] {
        assert!(
            impls_for(ty)
                .iter()
                .all(|h| h.contains("T:Default") && !h.contains("Clone")),
            "{ty}'s cycle shares its own premise only"
        );
    }
}

/// A cycle that alternates between traits shares its premises too, with the parameters mapped
/// through the cycle's bounds: `Term`'s `U: Clone` reaches `Expr` (the cycle's only `Eval` impl)
/// as `T: Clone`, through `Term: Size<T>`. `Term`'s `U: 'u` names a lifetime no bound fixes on
/// `Expr`'s side and stays where it is; a same-trait cycle beside it (`L`, `R`) keeps its own.
#[test]
fn cross_trait_cycle_members_share_their_mapped_premises() {
    let out = expand(parse_quote! {
        mod m {
            #[decycle] pub trait Eval<T> { fn eval(&self, t: &T) -> u32; }
            #[decycle] pub trait Size<T> { fn size(&self, t: &T) -> u32; }
            pub struct Expr; pub struct Term; pub struct L; pub struct R;
            impl<T> Eval<T> for Expr where Term: Size<T> { fn eval(&self, t: &T) -> u32 { 0 } }
            impl<'u, U> Size<U> for Term where Expr: Eval<U>, U: Clone + 'u {
                fn size(&self, t: &U) -> u32 { 0 }
            }
            impl<T> Eval<T> for L where R: Eval<T> { fn eval(&self, t: &T) -> u32 { 1 } }
            impl<T> Eval<T> for R where L: Eval<T>, T: Default { fn eval(&self, t: &T) -> u32 { 2 } }
        }
    });
    let squished: String = out.split_whitespace().collect();
    let impls_for = |ty: &str| -> Vec<&str> {
        squished
            .split("impl<")
            .filter(|chunk| chunk.contains(&format!("for{ty}")))
            .collect()
    };
    let expr = impls_for("Expr");
    assert!(!expr.is_empty(), "Expr has generated impls");
    assert!(
        expr.iter()
            .all(|h| h.contains("T:Clone") && !h.contains("'u") && !h.contains("Default")),
        "Expr's impls carry `Term`'s premise in its own terms, and only it: {expr:?}"
    );
    let term = impls_for("Term");
    assert!(
        term.iter()
            .all(|h| !h.contains("T:Clone") && !h.contains("Default")),
        "Term gains nothing: {term:?}"
    );
    for ty in ["L", "R"] {
        assert!(
            impls_for(ty)
                .iter()
                .all(|h| h.contains("T:Default") && !h.contains("Clone")),
            "{ty}'s cycle shares its own premise only"
        );
    }
}
