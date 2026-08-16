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
//! 3. the body contains a call that **resolves to the cycle's own trait method** — the same name,
//!    reached through a spelling that names a routed trait or `Self` (`<B as Eval>::eval(..)`,
//!    `Eval::eval(..)`, `Self::eval(..)`) or through a receiver rooted at `self` or at a cycle
//!    member (`self.eval(..)`, `B.eval(..)`) — so the callee's parameter at that position is
//!    provably the same by-value generic; and
//! 4. the argument at that position is `&expr` / `&mut expr` whose root binding is **tainted**: it is
//!    that by-value generic parameter, or a local whose initialiser is rooted at one.
//!
//! Condition 4 is what keeps `<B as Eval>::eval(&mut local_buf, …)` legal — that instantiates at a
//! *fixed* `&mut Vec<u8>`, which closes after one step. Condition 3 is what keeps
//! `<B as Eval>::eval_by_ref(&mut src, …)` legal, where the callee takes `&mut S` and `S` never grows.
//!
//! Condition 3 used to be *only* "a call to a method of the same name", which does not establish any
//! of what the diagnostic then asserts. `Helper.eval(&src)`, calling a **different trait's** method
//! `Aux::eval<S>(&self, src: &S)` — which takes `S` by REFERENCE, so nothing grows at all — was
//! rejected outright, as was a plain free function that happened to share the name. That is a false
//! positive with no escape hatch on a program that compiles and runs without `#[decycle]`, which is
//! strictly worse than the rustc diagnostic this pass exists to replace.
//!
//! Known gaps, all of them false *negatives* (the program still fails, just with rustc's diagnostic):
//!
//! * **cross-method recursion** — `A::f` reborrowing into `B::g`, which reborrows back into `A::f`.
//!   Condition 3 keys on the method name, so this is missed.
//! * **a call routed through a type rather than a trait** — `B::eval(..)`, or a receiver bound to a
//!   local (`let b = B; b.eval(..)`). Neither spelling proves the callee is the trait method (an
//!   inherent method of the same name shadows it, with a signature of its own), and guessing there
//!   is what produced the false positive above.
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
    Expr, ExprPath, FnArg, Ident, ImplItem, Item, ItemImpl, ItemMod, Pat, Path, Signature, Stmt,
    Type, UseTree,
};

/// What a call site has to name for its callee to be the cycle's own trait method — condition 3.
struct Callees {
    /// Idents of the traits `#[decycle]` routes through this module. A path segment naming one
    /// (`Eval::eval(..)`, `<B as Eval>::eval(..)`) names the trait method itself.
    routed_traits: HashSet<String>,
    /// The cycle members' type names, so a receiver spelled as one (`B.eval(..)`) is recognised.
    members: HashSet<String>,
}

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
    let callees = Callees {
        routed_traits: routed_trait_names(items, decycle),
        members: cyclic.clone(),
    };
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
                check_method(&f.sig, &f.block, &callees)?;
            }
        }
    }
    Ok(())
}

/// The idents of the traits routed through this module — `#[decycle] trait Tr` and
/// `#[decycle] use path::Tr` (under its local name).
///
/// A small re-derivation of `analysis`'s private `routed_traits`, kept here because this pass runs
/// before any of the engine's own bookkeeping exists and only needs the names.
fn routed_trait_names(items: &[Item], decycle: &Path) -> HashSet<String> {
    fn use_idents(tree: &UseTree, out: &mut HashSet<String>) {
        match tree {
            UseTree::Path(p) => use_idents(&p.tree, out),
            UseTree::Name(n) => {
                out.insert(n.ident.to_string());
            }
            UseTree::Rename(r) => {
                out.insert(r.rename.to_string());
            }
            UseTree::Group(g) => g.items.iter().for_each(|t| use_idents(t, out)),
            UseTree::Glob(_) => {}
        }
    }
    let mut out = HashSet::new();
    let Some(first) = decycle.segments.first() else {
        return out;
    };
    let decycle_crate = &first.ident;
    let marked =
        |attrs: &[syn::Attribute]| attrs.iter().any(|a| crate::is_decycle_attribute(a, decycle_crate));
    for item in items {
        match item {
            Item::Trait(t) if marked(&t.attrs) => {
                out.insert(t.ident.to_string());
            }
            Item::Use(u) if marked(&u.attrs) => use_idents(&u.tree, &mut out),
            _ => {}
        }
    }
    out
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

fn check_method(sig: &Signature, block: &syn::Block, callees: &Callees) -> syn::Result<()> {
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

    struct V<'a> {
        name: &'a Ident,
        positions: &'a HashSet<usize>,
        taint: &'a HashSet<String>,
        derived: &'a HashSet<String>,
        callees: &'a Callees,
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

        /// Does this path call name the cycle's own trait method (condition 3)?
        ///
        /// Only a spelling that names a **trait** — or `Self`, whose impl is the one being checked —
        /// establishes that the callee's signature is the trait's, and hence that its parameter at
        /// the flagged position is the same by-value generic. A bare single-segment path is a free
        /// function; a type-rooted path (`B::eval`) may be an inherent method with an unrelated
        /// signature.
        fn path_names_trait_method(&self, p: &ExprPath) -> bool {
            if !p.path.segments.last().is_some_and(|s| &s.ident == self.name) {
                return false;
            }
            let owner = match &p.qself {
                // `<B as Eval>::eval(..)` — the segments before `as`'s end name the trait.
                Some(q) => match q.position.checked_sub(1) {
                    Some(ix) => p.path.segments.get(ix),
                    // `<B>::eval(..)`: no trait named at all.
                    None => return false,
                },
                // `Eval::eval(..)`, `Self::eval(..)`, `path::to::Eval::eval(..)`.
                None => match p.path.segments.len().checked_sub(2) {
                    Some(ix) => p.path.segments.get(ix),
                    None => return false,
                },
            };
            owner.is_some_and(|s| {
                s.ident == "Self" || self.callees.routed_traits.contains(&s.ident.to_string())
            })
        }

        /// Is this method call's receiver one the cycle's own trait method can be reached through —
        /// `self` (or a projection of it), or a cycle member named outright (`B.eval(..)`)?
        fn receiver_reaches_cycle(&self, receiver: &Expr) -> bool {
            root_ident(receiver).is_some_and(|id| {
                id == "self" || self.callees.members.contains(&id.to_string())
            })
        }
    }
    impl<'ast> Visit<'ast> for V<'ast> {
        fn visit_expr(&mut self, e: &'ast Expr) {
            match e {
                // `<B as Eval>::eval(a, b)`, `Eval::eval(a, b)`, `Self::eval(a, b)`. A path call is
                // UFCS: its arguments line up ONE-TO-ONE with the callee's inputs, the receiver
                // included, whether or not the CALLER has one. (`base` used to be
                // `usize::from(has_receiver)`, which both mis-mapped a free call made from a method
                // and shifted a genuine UFCS call's arguments off by one.)
                Expr::Call(c) => {
                    if let Expr::Path(p) = &*c.func {
                        if self.path_names_trait_method(p) {
                            self.scan(&c.args, 0);
                        }
                    }
                }
                // `x.eval(a, b)` — the receiver is `x`, so `args[0]` is `sig.inputs[1]`.
                Expr::MethodCall(mc)
                    if &mc.method == self.name && self.receiver_reaches_cycle(&mc.receiver) =>
                {
                    self.scan(&mc.args, 1)
                }
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
        callees,
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
