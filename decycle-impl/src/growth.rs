//! Fail closed on **value-level instantiation growth**: a cyclic trait method that reborrows its own
//! by-value generic parameter at the recursive call.
//!
//! # The shape
//!
//! ```ignore
//! #[decycle] pub trait Eval {
//!     fn eval<S: Src>(src: S, depth: u32) -> u32;   // S is taken BY VALUE
//! }
//! impl Eval for A where B: Eval {
//!     fn eval<S: Src>(mut src: S, depth: u32) -> u32 {
//!         <B as Eval>::eval(&mut src, depth - 1)    // ← instantiates at `&mut S`
//!     }
//! }
//! ```
//!
//! `A::eval::<S>` calls `B::eval::<&mut S>`, which calls `A::eval::<&mut &mut S>`, … The
//! *obligation* cycle is broken fine — both engines do that — but the **instantiation set is
//! infinite**, and no obligation engine can help: the growth is in the monomorphisation, one `&mut`
//! layer per level.
//!
//! This is the value-level twin of the non-regular where-bound rejected in
//! [`ranked::finalize`](crate::ranked::finalize) (`A<Vec<X>>: Tr<Vec<X>>` on `impl<X> Tr<X> for A<X>`),
//! and it is rejected for the same reason: there is no compilable program in the class. Left alone it
//! surfaces as one of
//!
//! * `error: reached the recursion limit while instantiating fn::<&mut &mut &mut …>` — a
//!   monomorphisation limit with no error code, whose only span is the recursive call;
//! * `error[E0275]: overflow evaluating the requirement` against whatever blanket impl the reborrowed
//!   type needs (`impl<T: Src + ?Sized> Src for &mut T`), pointing at that innocent blanket rather
//!   than at the growth; or
//! * an ICE — `failed to resolve instance for …` — when only *some* call sites reborrow.
//!
//! None of the three names decycle, the growing parameter, or the fix. Hence this pre-pass.
//!
//! # What it detects, and what it does not
//!
//! Detection is syntactic and deliberately narrow, because a false positive rejects a working
//! program. All of the following must hold:
//!
//! 1. the impl's self type is a **cycle member** (per [`crate::analysis::cyclic_subgraph`]);
//! 2. the method has a **method-level generic type parameter** (or an argument-position `impl Trait`,
//!    which is one anonymously) taken **by value** as an argument;
//! 3. the body contains a call to a method of **the same name** — so the callee's parameter at that
//!    position is provably the same by-value generic — and
//! 4. the argument at that position is `&expr` / `&mut expr` whose root binding is **tainted**: it is
//!    that by-value generic parameter, or a local whose initialiser is rooted at one.
//!
//! Condition 4 is what keeps `<B as Eval>::eval(&mut local_buf, …)` legal — that instantiates at a
//! *fixed* `&mut Vec<u8>`, which closes after one step. Condition 3 is what keeps
//! `<B as Eval>::eval_by_ref(&mut src, …)` legal, where the callee takes `&mut S` and `S` never grows.
//!
//! Known gaps, all of them false *negatives* (the program still fails, just with rustc's diagnostic):
//!
//! * **cross-method recursion** — `A::f` reborrowing into `B::g`, which reborrows back into `A::f`.
//!   Condition 3 keys on the method name, so this is missed.
//! * **non-reference wrappers** — passing `Wrap(src)` rather than `&mut src` grows identically, but
//!   only `&`/`&mut` is recognised, since a call or struct literal at that position is far more often
//!   a legitimate fixed type.
//! * **taint through a non-`let` binding** — a `match`/`if let` arm binding, or a local whose
//!   initialiser mentions the parameter somewhere other than at its root.
//!
//! # The type-annotation escape hatch
//!
//! Taint follows `let` chains, and a *conversion* (`let s = src.into_src();`) is the spelling real code
//! uses — so the chain has to peel method calls, not only projections. But a method call's return type
//! need not involve the receiver at all (`let n = src.tick();` is a plain `u32`), so peeling them could
//! taint a binding whose type is in fact fixed, and reject a working program.
//!
//! An **explicit type annotation on the local settles it**: `let n: u32 = src.tick();` is untainted,
//! because the annotation says the type outright and does not mention any of the method's generic
//! parameters. `let s: S = ..` stays tainted. So every false positive this heuristic can produce is
//! fixable by the user in one line, and the diagnostic says so.

use crate::analysis;
use std::collections::HashSet;
use syn::visit::Visit;
use syn::{
    Expr, FnArg, Ident, ImplItem, Item, ItemImpl, ItemMod, Pat, Path, Signature, Stmt, Type,
};

/// Reject any value-level instantiation growth reachable in `module`. Engine-independent — the growth
/// defeats ranked and structural alike — so both entry points call it before doing any work.
pub(crate) fn check_value_generic_growth(module: &ItemMod, decycle: &Path) -> syn::Result<()> {
    let Some((_, items)) = module.content.as_ref() else {
        return Ok(());
    };
    let cyclic = cycle_member_names(module, decycle);
    if cyclic.is_empty() {
        return Ok(());
    }
    for item in items {
        let Item::Impl(im) = item else { continue };
        if im.trait_.is_none() {
            continue;
        }
        if !self_ident(im).is_some_and(|id| cyclic.contains(&id.to_string())) {
            continue;
        }
        for ii in &im.items {
            if let ImplItem::Fn(f) = ii {
                check_method(&f.sig, &f.block)?;
            }
        }
    }
    Ok(())
}

/// The cycle members' type names. An empty set (no cycle at all) short-circuits the whole pass.
fn cycle_member_names(module: &ItemMod, decycle: &Path) -> HashSet<String> {
    use crate::safegraph::graph::Graph;
    let graph = analysis::cyclic_subgraph(&analysis::analyze_module(module, decycle));
    graph.nodes().map(|n| n.to_string()).collect()
}

fn self_ident(im: &ItemImpl) -> Option<&Ident> {
    match &*im.self_ty {
        Type::Path(tp) => tp.path.segments.last().map(|s| &s.ident),
        _ => None,
    }
}

/// The positions of `sig`'s arguments that are a **by-value** occurrence of one of the method's own
/// generic type parameters, paired with the binding ident when the pattern is a plain `x` / `mut x`.
///
/// Indices are into `sig.inputs`, so a receiver occupies index 0 when present — the call-site walk
/// converts to that space rather than the other way round.
fn by_value_generic_args(sig: &Signature) -> Vec<(usize, Option<Ident>)> {
    let own: HashSet<String> = sig
        .generics
        .type_params()
        .map(|p| p.ident.to_string())
        .collect();
    let mut out = Vec::new();
    for (i, input) in sig.inputs.iter().enumerate() {
        let FnArg::Typed(pt) = input else { continue };
        let hit = match &*pt.ty {
            // `impl Trait` in argument position IS a method generic, just anonymous — and it is the
            // spelling that actually bit (`fn parse(stream: impl IntoParseStream)`).
            Type::ImplTrait(_) => true,
            Type::Path(tp) => {
                tp.qself.is_none()
                    && tp.path.segments.len() == 1
                    && own.contains(&tp.path.segments[0].ident.to_string())
            }
            _ => false,
        };
        if hit {
            let bind = match &*pt.pat {
                Pat::Ident(pi) => Some(pi.ident.clone()),
                _ => None,
            };
            out.push((i, bind));
        }
    }
    out
}

/// Peel a **place** expression down to the identifier it is rooted at: `&mut *s.inner` → `s`.
///
/// Only projections are peeled. A call, macro, or struct literal is not a projection — its result is a
/// fresh value whose type need not involve the parameter at all — so those return `None`. This is the
/// strict form, used on the flagged *argument*: `recurse(&mut make_buffer(), ..)` must not be flagged.
fn root_ident(mut e: &Expr) -> Option<&Ident> {
    loop {
        e = match e {
            Expr::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => {
                return Some(&p.path.segments[0].ident)
            }
            Expr::Paren(x) => &x.expr,
            Expr::Group(x) => &x.expr,
            Expr::Field(x) => &x.base,
            Expr::Index(x) => &x.expr,
            Expr::Reference(x) => &x.expr,
            Expr::Try(x) => &x.expr,
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => &u.expr,
            _ => return None,
        };
    }
}

/// [`root_ident`], but also peeling a **method-call receiver**, `?` and `.await` — the "derived from"
/// relation rather than the "same place as" one. Used only to propagate taint along `let` chains,
/// where `let s = src.into_src();` is the spelling that matters. Kept separate from [`root_ident`]
/// because the looser relation would be wrong on the flagged argument.
fn taint_root(mut e: &Expr) -> Option<&Ident> {
    loop {
        e = match e {
            Expr::Path(p) if p.qself.is_none() && p.path.segments.len() == 1 => {
                return Some(&p.path.segments[0].ident)
            }
            Expr::Paren(x) => &x.expr,
            Expr::Group(x) => &x.expr,
            Expr::Field(x) => &x.base,
            Expr::Index(x) => &x.expr,
            Expr::Reference(x) => &x.expr,
            Expr::Try(x) => &x.expr,
            Expr::MethodCall(x) => &x.receiver,
            Expr::Await(x) => &x.base,
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => &u.expr,
            _ => return None,
        };
    }
}

/// Does `ty` name any of `generics`? An annotated local is tainted only when its annotation still
/// mentions a method generic — that is what makes the annotation a usable opt-out.
fn mentions_any(ty: &Type, generics: &HashSet<String>) -> bool {
    struct V<'a>(&'a HashSet<String>, bool);
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_ident(&mut self, i: &'ast Ident) {
            if self.0.contains(&i.to_string()) {
                self.1 = true;
            }
        }
    }
    let mut v = V(generics, false);
    v.visit_type(ty);
    v.1
}

/// Grow the tainted set — bindings whose type is derived from a by-value generic parameter — over the
/// body's `let` statements, to a fixpoint.
///
/// This is what catches the indirect spelling, which is the one real code hits:
///
/// ```ignore
/// fn descend(s: impl IntoParseStream, depth: u32) {
///     let mut s = s.into_parse_stream();   // `s` re-bound to a type derived from the parameter
///     descend(&mut s, depth - 1);          // still grows
/// }
/// ```
fn tainted_bindings(
    block: &syn::Block,
    seeds: &[Ident],
    generics: &HashSet<String>,
) -> (HashSet<String>, HashSet<String>) {
    let mut taint: HashSet<String> = seeds.iter().map(|i| i.to_string()).collect();
    // Locals (as opposed to the parameters themselves) that taint reached, so the diagnostic can offer
    // the annotation escape hatch only where it applies.
    let mut derived: HashSet<String> = HashSet::new();
    // A `let` may re-bind a name the seed set already holds, and a later `let` may depend on an
    // earlier one, so iterate rather than making one pass.
    loop {
        let before = taint.len();
        collect_lets(block, &mut taint, &mut derived, generics);
        if taint.len() == before {
            return (taint, derived);
        }
    }
}

fn collect_lets(
    block: &syn::Block,
    taint: &mut HashSet<String>,
    derived: &mut HashSet<String>,
    generics: &HashSet<String>,
) {
    struct V<'a> {
        taint: &'a mut HashSet<String>,
        derived: &'a mut HashSet<String>,
        generics: &'a HashSet<String>,
    }
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_stmt(&mut self, s: &'ast Stmt) {
            if let Stmt::Local(local) = s {
                // `let x = ..` / `let x: T = ..`; syn models the annotation as `Pat::Type`.
                let (name, ann) = match &local.pat {
                    Pat::Ident(pi) => (Some(&pi.ident), None),
                    Pat::Type(pt) => match &*pt.pat {
                        Pat::Ident(pi) => (Some(&pi.ident), Some(&*pt.ty)),
                        _ => (None, None),
                    },
                    _ => (None, None),
                };
                if let (Some(name), Some(init)) = (name, &local.init) {
                    // An annotation that names no method generic states a fixed type: opt out.
                    let annotated_fixed = ann.is_some_and(|t| !mentions_any(t, self.generics));
                    if !annotated_fixed
                        && taint_root(&init.expr)
                            .is_some_and(|r| self.taint.contains(&r.to_string()))
                    {
                        self.taint.insert(name.to_string());
                        self.derived.insert(name.to_string());
                    }
                }
            }
            syn::visit::visit_stmt(self, s);
        }
    }
    V {
        taint,
        derived,
        generics,
    }
    .visit_block(block);
}

fn check_method(sig: &Signature, block: &syn::Block) -> syn::Result<()> {
    let by_value = by_value_generic_args(sig);
    if by_value.is_empty() {
        return Ok(());
    }
    let seeds: Vec<Ident> = by_value.iter().filter_map(|(_, b)| b.clone()).collect();
    if seeds.is_empty() {
        // Nothing nameable to taint (a destructuring pattern on a generic-typed argument), so there is
        // no way to tell a reborrow of it from a reborrow of anything else. Stay quiet.
        return Ok(());
    }
    let own_generics: HashSet<String> = sig
        .generics
        .type_params()
        .map(|p| p.ident.to_string())
        .collect();
    let (taint, derived) = tainted_bindings(block, &seeds, &own_generics);
    let positions: HashSet<usize> = by_value.iter().map(|(i, _)| *i).collect();
    let has_receiver = sig.receiver().is_some();

    struct V<'a> {
        name: &'a Ident,
        positions: &'a HashSet<usize>,
        taint: &'a HashSet<String>,
        derived: &'a HashSet<String>,
        has_receiver: bool,
        found: Option<(&'a Expr, bool)>,
    }
    impl<'a> V<'a> {
        /// `args[j]` occupies `sig.inputs[base + j]`.
        fn scan(
            &mut self,
            args: &'a syn::punctuated::Punctuated<Expr, syn::Token![,]>,
            base: usize,
        ) {
            for (j, a) in args.iter().enumerate() {
                if !self.positions.contains(&(base + j)) {
                    continue;
                }
                // Only a reference GROWS. Passing the value on unchanged keeps the same `S` and
                // terminates, which is the whole point of a by-value generic being legal here.
                let Expr::Reference(r) = a else { continue };
                let Some(root) = root_ident(&r.expr) else {
                    continue;
                };
                if self.taint.contains(&root.to_string()) && self.found.is_none() {
                    self.found = Some((a, self.derived.contains(&root.to_string())));
                }
            }
        }
    }
    impl<'ast> Visit<'ast> for V<'ast> {
        fn visit_expr(&mut self, e: &'ast Expr) {
            match e {
                // `<B as Eval>::eval(a, b)`, `B::eval(a, b)`, `Eval::eval(a, b)`. A path call to a
                // method WITH a receiver is UFCS, so the receiver is `args[0]`.
                Expr::Call(c) => {
                    if let Expr::Path(p) = &*c.func {
                        if p.path
                            .segments
                            .last()
                            .is_some_and(|s| &s.ident == self.name)
                        {
                            self.scan(&c.args, usize::from(self.has_receiver));
                        }
                    }
                }
                // `x.eval(a, b)` — the receiver is `x`, so `args[0]` is `sig.inputs[1]`.
                Expr::MethodCall(mc) if &mc.method == self.name => self.scan(&mc.args, 1),
                _ => {}
            }
            syn::visit::visit_expr(self, e);
        }
    }

    let mut v = V {
        name: &sig.ident,
        positions: &positions,
        taint: &taint,
        derived: &derived,
        has_receiver,
        found: None,
    };
    v.visit_block(block);

    if let Some((call, via_local)) = v.found {
        let name = &sig.ident;
        // `new_spanned`, not `new`: the caret should cover the whole `&mut src`, and a `Span` built
        // from a multi-token expression collapses to its first token.
        return Err(syn::Error::new_spanned(
            call,
            format!(
                "decycle: this recursive call reborrows `{name}`'s own by-value generic parameter, so \
                 each level instantiates it one `&mut` deeper (`{name}::<S>` → `{name}::<&mut S>` → \
                 `{name}::<&mut &mut S>` → …). The obligation cycle is breakable; this instantiation \
                 cycle is not — the growth is in the monomorphisation, and no engine can close it \
                 (rustc reports it as \"reached the recursion limit while instantiating\", or as an \
                 E0275 against whatever blanket impl `&mut _` needs).\n\
                 \n\
                 Take the parameter by reference instead, so the recursive call reborrows to the SAME \
                 type and `S` is a fixed point:\n\
                 \n    fn {name}<S: ..>(arg: &mut S, ..)      // signature\n\
                 \n    {name}(&mut *arg, ..)                  // recursive call\n\
                 \n\
                 If the by-value signature is part of a public API, keep it as a provided entry point \
                 that delegates once to the by-reference method. Erasing the parameter behind one \
                 fixed type (`&mut dyn Trait`) also closes it, at the cost of virtual dispatch.{}",
                if via_local {
                    "\n\nThis was reached through a local derived from the parameter. If that local's \
                     type does NOT in fact involve the parameter, give it an explicit type annotation \
                     (`let x: ConcreteType = ..`) and this check will accept it."
                } else {
                    ""
                }
            ),
        ));
    }
    Ok(())
}
