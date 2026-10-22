use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro_error::*;
use std::collections::HashSet;
use syn::spanned::Spanned;
use syn::*;
use crate::helper::type_head_ident;
use template_quote::quote;

/// A `super::super::…`-rooted path (2+ leading `super` segments) written by the user
/// inside a `#[decycle]` module breaks once `finalize` re-emits the module's items
/// nested inside `shadowing_module`/`shadowing_module::ranked_traits` — each extra
/// module layer shifts what `super::super` actually points at, so the path silently
/// resolves to the wrong place (or fails to resolve) rather than what the user wrote. A
/// single `super::` is fine (finalize's own generated code already relies on that
/// exact depth to reach back out of its wrapper modules); only depth >= 2 is rejected.
fn check_no_deep_super_paths(contents: &[Item]) {
    use syn::visit::Visit;
    struct Visitor;
    impl<'ast> Visit<'ast> for Visitor {
        fn visit_path(&mut self, path: &'ast Path) {
            if path.leading_colon.is_none() {
                let super_count = path
                    .segments
                    .iter()
                    .take_while(|seg| seg.ident == "super")
                    .count();
                if super_count >= 2 {
                    abort!(
                        path,
                        "paths with multiple super segments are not supported inside #[decycle] modules; use crate::-rooted paths"
                    );
                }
            }
            syn::visit::visit_path(self, path);
        }
    }
    let mut visitor = Visitor;
    for item in contents {
        visitor.visit_item(item);
    }
}

fn check_submodule(module: &ItemMod, decycle_crate: &Ident) {
    use syn::visit::Visit;
    struct Visitor<'a> {
        decycle_crate: &'a Ident,
    }
    impl<'ast, 'a> Visit<'ast> for Visitor<'a> {
        fn visit_attribute(&mut self, i: &'ast syn::Attribute) {
            if crate::is_decycle_attribute(i, self.decycle_crate) {
                abort!(&i, "#[decycle] is not supported in nested modules")
            }
            syn::visit::visit_attribute(self, i);
        }
    }
    Visitor { decycle_crate }.visit_item_mod(module);
}

fn process_trait_path(item: &Item) -> Vec<Path> {
    match item {
        Item::Trait(ItemTrait { ident, .. }) => {
            vec![crate::ident_to_path(ident)]
        }
        Item::TraitAlias(item_trait_alias) => {
            // A trait alias has no body to carry through the macro ping-pong (there's no
            // `ItemTrait` to embed), so `#[decycle]` on one silently produced a bogus
            // working-list entry with nothing behind it. Reject it cleanly instead.
            abort!(
                &item_trait_alias.ident,
                "#[decycle] is not supported on a trait alias"
            )
        }
        Item::Use(ItemUse { tree, .. }) => {
            fn process_use_tree(tree: &UseTree) -> Vec<Path> {
                match tree {
                    UseTree::Path(UsePath { tree, .. }) => process_use_tree(tree),
                    UseTree::Name(UseName { ident })
                    | UseTree::Rename(UseRename { rename: ident, .. }) => {
                        vec![crate::ident_to_path(ident)]
                    }
                    UseTree::Glob(use_glob) => {
                        abort!(use_glob, "glob is not supported in #[decycle] use")
                    }
                    UseTree::Group(UseGroup { items, .. }) => {
                        items.iter().flat_map(process_use_tree).collect()
                    }
                }
            }
            process_use_tree(tree)
        }
        _ => unreachable!(),
    }
}

/// `(original_ident, local_alias)` for every `UseRename` (`use path::T as R;`) reachable
/// from a `#[decycle] use` item — see `finalize::TraitRename`/L-C1. A plain `UseName`
/// (`use path::T;`) needs no entry: the local name already matches the original.
fn collect_trait_renames(item: &Item) -> Vec<(Ident, Ident)> {
    fn walk(tree: &UseTree, out: &mut Vec<(Ident, Ident)>) {
        match tree {
            UseTree::Path(UsePath { tree, .. }) => walk(tree, out),
            UseTree::Rename(UseRename { ident, rename, .. }) => {
                out.push((ident.clone(), rename.clone()));
            }
            UseTree::Name(_) | UseTree::Glob(_) => (),
            UseTree::Group(UseGroup { items, .. }) => {
                for item in items {
                    walk(item, out);
                }
            }
        }
    }
    let Item::Use(ItemUse { tree, .. }) = item else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

pub(crate) fn is_local_impl_bound_target(ty: &Type, impl_type_params: &HashSet<Ident>) -> bool {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return false;
    };
    if path.segments.len() != 1 {
        return false;
    }
    let ident = &path.segments[0].ident;
    ident == "Self" || impl_type_params.contains(ident)
}

/// Does `im` implement a routed trait in the spelling the engine takes over — a single segment after
/// `self::`-normalisation?
fn impls_routed_trait(im: &ItemImpl, all_traits: &HashSet<Ident>) -> bool {
    im.trait_
        .as_ref()
        .and_then(|(_, trait_path, _)| crate::names::routed_ident(trait_path))
        .is_some_and(|ident| all_traits.contains(ident))
}

/// `impl Tr for self::Term` → `impl Tr for Term`, the spelling a cycle member is matched by — unless
/// the bare name is one of the impl's own type parameters, which would then shadow the type.
fn bare_self_type(im: &mut ItemImpl) {
    let Type::Path(TypePath { qself: None, path }) = &mut *im.self_ty else {
        return;
    };
    let mut bare = path.clone();
    crate::names::strip_leading_self(&mut bare);
    let shadowed = bare.segments.len() == 1
        && im
            .generics
            .params
            .iter()
            .any(|p| matches!(p, GenericParam::Type(t) if t.ident == bare.segments[0].ident));
    if !shadowed {
        *path = bare;
    }
}

/// Is `ty` spelled by a path other than a bare name or its no-op `self::` form (`crate::m::Term`,
/// `super::Term`, `::dep::Term`)? Such a self type names the item itself, the opt-out.
fn self_ty_spelled_by_path(ty: &Type) -> bool {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return false;
    };
    crate::names::bare_ident(path).is_none()
}

fn has_assoc_constraints(path: &Path) -> bool {
    let Some(last_segment) = path.segments.last() else {
        return false;
    };
    let PathArguments::AngleBracketed(args) = &last_segment.arguments else {
        return false;
    };
    args.args.iter().any(|arg| {
        matches!(
            arg,
            GenericArgument::AssocType(_)
                | GenericArgument::AssocConst(_)
                | GenericArgument::Constraint(_)
        )
    })
}

/// The help text must name exactly what `is_local_impl_bound_target` accepts: `Self`, or
/// one of THIS impl's own generic type parameters — not any type merely defined inside the
/// `#[decycle]` module (a module-local struct/enum bound target is not accepted by the
/// check, so the old text promised something the rule didn't actually allow).
fn local_types_help_message(impl_type_params: &HashSet<Ident>) -> String {
    if impl_type_params.is_empty() {
        return "use `Self` or one of this `impl`'s own type parameters".to_owned();
    }
    let mut params: Vec<&Ident> = impl_type_params.iter().collect();
    params.sort_by_key(|ident| ident.to_string());
    let types = params
        .iter()
        .map(|ident| format!("`{ident}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("use `Self` or one of this `impl`'s own type parameters, such as {types}")
}

fn validate_impl_where_bounds(
    item_impl: &ItemImpl,
    all_traits: &HashSet<Ident>,
    cycle_self_heads: &HashSet<Ident>,
) {
    let impl_type_params = crate::names::type_param_idents(&item_impl.generics);

    let Some(where_clause) = &item_impl.generics.where_clause else {
        return;
    };

    for pred in &where_clause.predicates {
        let WherePredicate::Type(PredicateType {
            bounded_ty, bounds, ..
        }) = pred
        else {
            continue;
        };
        if is_local_impl_bound_target(bounded_ty, &impl_type_params) {
            continue;
        }
        for bound in bounds {
            let TypeParamBound::Trait(TraitBound { path, .. }) = bound else {
                continue;
            };
            let mut path = path.clone();
            let was_self_qualified = path.segments.len() > 1;
            crate::names::strip_leading_self(&mut path);
            // Single-segment match only (after self::-normalization) — a multi-segment
            // path (`some::mod::Foo`) merely sharing a last segment with a #[decycle]
            // trait is a DIFFERENT item; matching on the last segment alone was a false
            // positive. NOTE: a still-multi-segment, qualified reference to a #[decycle]
            // trait (`super::Foo`, `crate::mod::Foo`) is intentionally NOT flagged here —
            // it's the documented way to bind a FOREIGN (non-cyclic) type to the ORIGINAL,
            // un-ranked trait in a side-bound (`Foreign: super::Foo`; `nesting::LiftRelative`
            // lifts the depth-fragile `super::` spelling so it survives re-emission); only
            // the bare/`self::`-qualified form participates in ranking at all, so there's
            // no reliable syntactic way to tell "meant to be ranked, mis-qualified" apart
            // from this deliberate opt-out.
            if path.segments.len() != 1 {
                continue;
            }
            let last_segment = &path.segments[0];
            if all_traits.contains(&last_segment.ident) {
                // A bare cyclic bound `X: Tr` is rank-lowered to `X: TrRanked<Rank>`, which resolves
                // only if `X`'s head type has a ranked impl in this module — `Self`/an impl type-param
                // (already skipped above) or a cycle self type (`Right`, `Wrap<&A>`). Computed FIRST
                // because it gates BOTH aborts below: a rank-lowerable target is fully handled by the
                // ranked pipeline even when the bound carries associated-type constraints
                // (`TraitReplacer::try_replace_path` moves the segment's `PathArguments` wholesale onto
                // the ranked path and splices `Rank` positionally, so a binding like `Span = X` survives
                // rank-lowering intact — `Stmt<S>: TrRanked<Rank, Span = X>` — and the ranked twin trait
                // declares the associated type verbatim via `process_trait_item_for_ranked`).
                //
                // PRECONDITION on the caller (documented, not enforced): an impl generic that is
                // INVENTED (appears in no field/self type) must be grounded by at least one
                // binding-carrying bound on a NON-cycle-member target that survives
                // `remove_cyclic_bounds` — the rank floor necessarily drops cycle premises, so a param
                // pinned only via cycle-member bindings has nothing constraining it at rank `()`
                // (the resulting E0207 there is not fixable by decycle). Callers achieve this by
                // putting such leaf bounds on a supertrait alias whose own ident is not a decycle
                // trait, so they pass through the cyclic-bound sweep untouched.
                //
                // Matched by the BARE (or `self::`-qualified) spelling: `crate::other::Stmt` is a
                // different item from the local `Stmt`, and reading it as rank-lowerable skipped
                // the abort below and let `finalize` rank-lower a foreign type — a wall of
                // unprovable `TrRanked<…>` errors in place of one legible rejection.
                let head_ok = crate::names::local_type_head_ident(bounded_ty)
                    .is_some_and(|h| cycle_self_heads.contains(&h));
                // F6: the earlier wording ("...on non-local type") described what the check
                // rejects, but the check doesn't actually key on locality — a bound on a
                // module-LOCAL struct/enum (anything other than `Self` or one of this impl's
                // own type parameters) is rejected exactly the same way. State what's
                // ACCEPTED instead, which is unambiguous either way. Only reached when the target
                // is NOT rank-lowerable (see `head_ok` above — a lowerable target carries its
                // bindings through the rewrite fine); checked before the un-lowerable-head abort
                // below so a bound with assoc constraints gets the more specific message.
                if has_assoc_constraints(&path) && !head_ok {
                    let help_message = local_types_help_message(&impl_type_params);
                    abort!(
                        path,
                        "associated-type constraints in #[decycle] impl where-clauses are only supported on `Self` or the impl's own type parameters";
                        help = bounded_ty.span() => "{}", help_message
                    );
                }
                // M2: a BARE-spelled bound on a foreign/non-participant head is a leaf premise
                // on the ORIGINAL trait, not an edge — `remove_cyclic_bounds` keeps it, and
                // `requalify_foreign_premises` (finalize.rs) now carries it through the
                // `shadowing_module` rewrites un-lowered, so the impl compiles whenever the
                // user supplies a matching impl of the original trait (and without one, rustc
                // reports the plain `Vec<Stmt>: Tr` obligation — never a `TrRanked<…>` wall).
                // No abort for that form anymore.
                //
                // The `self::`-qualified spelling is different: it is depth-fragile, so
                // `remove_cyclic_bounds` classifies it as cyclic REGARDLESS of its target and
                // the rewrite still rank-lowers it — for a non-lowerable head that can only
                // produce the unprovable-`TrRanked` wall, so keep rejecting it up front with
                // the legible message on the user's own bound.
                if !head_ok && was_self_qualified {
                    abort!(
                        bounded_ty,
                        "decycle: this cyclic bound's target is not a type the ranked engine can rank-lower — its head is a container or non-cycle type (e.g. `Box<Stmt>`), so the ranked chain has no floor impl to descend through";
                        help = "use `#[decycle(structural)]` (it forwards a wrapped bound via a blanket `impl<T: Tr> Tr for Box<T>`), spell the trait bare to state the bound as a premise on the original trait, or bound a bare cycle type instead"
                    );
                }
            }
        }
    }
}

/// The cyclic bounds of `im` for [`crate::premise_flow`]: each bound (`is_edge`) naming a routed
/// trait on an adopted head (the head of a peeled bound is the member it obligates; `Self` is `im`'s
/// own self type), with the impls of `impls` of that trait for that head, which may discharge it. A
/// bound on one of `im`'s own type parameters names no impl in particular and is no such edge.
fn ranked_cycle_edges(
    im: &ItemImpl,
    impls: &[&ItemImpl],
    all_traits: &HashSet<Ident>,
    adopted_heads: &HashSet<Ident>,
    is_edge: &dyn Fn(&ItemImpl, &WherePredicate) -> bool,
) -> Vec<crate::premise_flow::CycleEdge> {
    let type_params = crate::names::type_param_idents(&im.generics);
    let binder_of = |b: Option<&BoundLifetimes>| -> Vec<Lifetime> {
        b.into_iter()
            .flat_map(|b| &b.lifetimes)
            .filter_map(|p| match p {
                GenericParam::Lifetime(l) => Some(l.lifetime.clone()),
                _ => None,
            })
            .collect()
    };
    let trait_ident_of = |path: &Path| crate::names::bare_ident(path).cloned();
    let mut out = Vec::new();
    for pred in im.generics.where_clause.iter().flat_map(|wc| &wc.predicates) {
        let WherePredicate::Type(pt) = pred else {
            continue;
        };
        if !is_edge(im, pred) {
            continue;
        }
        let target = match &pt.bounded_ty {
            Type::Path(TypePath { qself: None, path }) if path.is_ident("Self") => {
                (*im.self_ty).clone()
            }
            other => other.clone(),
        };
        let Some(head) = crate::names::local_type_head_ident(&target) else {
            continue;
        };
        if type_params.contains(&head) || !adopted_heads.contains(&head) {
            continue;
        }
        for bound in &pt.bounds {
            let TypeParamBound::Trait(tb) = bound else {
                continue;
            };
            let Some(trait_ident) = trait_ident_of(&tb.path).filter(|t| all_traits.contains(t))
            else {
                continue;
            };
            let candidates = impls
                .iter()
                .enumerate()
                .filter(|(_, j)| {
                    j.trait_
                        .as_ref()
                        .and_then(|(_, path, _)| trait_ident_of(path))
                        .is_some_and(|t| t == trait_ident)
                        && crate::names::local_type_head_ident(&j.self_ty)
                            .is_some_and(|h| h == head)
                })
                .map(|(k, _)| k)
                .collect();
            let mut binder = binder_of(pt.lifetimes.as_ref());
            binder.extend(binder_of(tb.lifetimes.as_ref()));
            out.push(crate::premise_flow::CycleEdge {
                target: target.clone(),
                trait_path: tb.path.clone(),
                binder,
                candidates,
            });
        }
    }
    out
}

/// For each impl of one routed trait, which of its siblings' premises it may inherit — the
/// obligation-reachability scoping behind [`crate::ranked::sharing::share_side_predicates`].
///
/// `members` are the adopted impls of `trait_ident`, in group order; the result is indexed the same
/// way: `sources[i]` lists the indices of the members whose premises member `i`'s rank chain can
/// actually be obligated by, i.e. the members reachable from `i` over the trait's own cyclic-bound
/// edges. Within one strongly connected component every member reaches every other, so a genuine
/// cycle keeps the full union it always had; an impl with no obligation path to a sibling — a
/// standalone acyclic impl, a member of a *different* disjoint cycle of the same trait, or two
/// unrelated instantiations of one generic trait on disjoint types — inherits nothing from it.
///
/// An edge exists when a member's where-predicate carries a bound naming `trait_ident` in the
/// spelling the engine ranks (bare or `self::`-qualified — matching `peel`/`validate`), and its
/// target's head is another member's self-type head. Consistent with the rest of the engine (and
/// with `analysis.rs`'s graph), nodes are identified by **head ident**: `impl Tr for A<u8>` and
/// `impl Tr for A<i32>` share a node, so a bound `A<i32>: Tr` conservatively reaches both — the
/// bound-to-impl matching is not finer-grained anywhere else either. A target that is `Self` or one
/// of the impl's own generic params names no other member (and a param *shadowing* a member's name
/// must not fabricate an edge), so it contributes none.
fn sharing_sources(members: &[&ItemImpl], trait_ident: &Ident) -> Vec<Vec<usize>> {
    // Node key per member: its self type's head ident. An un-headed self type (`&A`, `(A, B)`)
    // gets a synthetic key no bound can name: it can obligate others, never be obligated by ident.
    let keys: Vec<String> = members
        .iter()
        .enumerate()
        .map(|(i, im)| {
            type_head_ident(&im.self_ty)
                .map(|id| id.to_string())
                .unwrap_or_else(|| format!("#unheaded_{i}"))
        })
        .collect();
    let member_heads: HashSet<&String> = keys.iter().collect();
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (i, im) in members.iter().enumerate() {
        let impl_type_params = crate::names::type_param_idents(&im.generics);
        let Some(where_clause) = &im.generics.where_clause else {
            continue;
        };
        for pred in &where_clause.predicates {
            let WherePredicate::Type(pt) = pred else {
                continue;
            };
            if is_local_impl_bound_target(&pt.bounded_ty, &impl_type_params) {
                continue;
            }
            let names_this_trait = pt.bounds.iter().any(|b| {
                matches!(b, TypeParamBound::Trait(TraitBound { path, .. })
                    if crate::names::routed_ident(path) == Some(trait_ident))
            });
            if !names_this_trait {
                continue;
            }
            // Peeling already ran, so a surviving cyclic bound's own head names the member it
            // obligates (or the bound is local, handled above).
            //
            // `local_type_head_ident`, NOT `type_head_ident`: a member is named by a bare (or
            // `self::`-qualified) ident only. Keying on the last segment made a premise on a
            // FOREIGN type (`crate::other::B: Tr`) fabricate the edge `A -> B` against the local
            // member that merely shares that name, and predicate sharing then unioned the
            // sibling's own leaf premises into `A` — `A<NotClone>: Tr` stopped holding, while
            // renaming the foreign type made the identical program compile. Same rule as
            // `peel::cycle_types_within`.
            let Some(head) = crate::names::local_type_head_ident(&pt.bounded_ty) else {
                continue;
            };
            let head = head.to_string();
            if member_heads.contains(&head) {
                let edge = (keys[i].clone(), head);
                if seen.insert(edge.clone()) {
                    edges.push(edge);
                }
            }
        }
    }
    // The reachability walk, over the crate's own graph machinery (`crate::graph`, which backs
    // `crate::analysis` too), by head name.
    let mut names: Vec<String> = member_heads.iter().map(|s| s.to_string()).collect();
    names.sort();
    let position = |name: &String| {
        names
            .binary_search(name)
            .expect("every key and every edge endpoint is a node")
    };
    let graph = crate::graph::compose(
        names.iter().cloned(),
        edges.iter().map(|(from, to)| (position(from), position(to), ())),
    );
    let reach = crate::graph::reachable(&graph);
    (0..members.len())
        .map(|i| {
            let reachable = &reach[position(&keys[i])];
            (0..members.len())
                .filter(|j| *j != i && reachable.contains(&position(&keys[*j])))
                .collect()
        })
        .collect()
}

/// Apply the ranked `#[decycle]` transformation to `module`, programmatically.
///
/// `decycle` is the path to the decycle crate (its leading segment names the crate, used to
/// recognise `#[<crate>::decycle]` on inner items). `recurse_level` sets the compile-time expansion
/// depth; `support_infinite_cycle` toggles the runtime re-entry registry (unbounded depth) versus a
/// fixed-depth floor. For macro authors wrapping `#[decycle]`; most users should use the attribute.
pub fn process_module(
    module: ItemMod,
    decycle: &Path,
    recurse_level: usize,
    support_infinite_cycle: bool,
) -> TokenStream {
    process_module_inner(module, None, decycle, recurse_level, support_infinite_cycle)
}

/// [`process_module`] for a module whose absolute path is `module_path` — the programmatic form of
/// `#[decycle(module = path)]`.
///
/// `module_path` must be absolute (`crate::..`, `$crate::..` or `::..`), without generic arguments,
/// and end in the module's own name; anything else aborts (through `proc_macro_error`). The cycle is
/// solved on the module as written, where only a bare (or `self::`) name is the module's; once it
/// is, the items re-emitted as written (everything but the impls the engine takes over — an impl of
/// a `#[decycle]` trait that names the trait or its self type by a path is the opt-out, re-emitted
/// and qualified too) have each bare or `self::` name of a type or trait declared in the module
/// spelled `<module_path>::X`, so an `#[announce]` on one of them sees absolute paths only. A check
/// that `module_path` names this module is emitted into it (an error at the path otherwise). A
/// bound that names one of the module's own types by a path (`<module_path>::C: Tr`) and is certain
/// to overflow, because `C`'s impl leads back to it, is reported at the bound (through
/// `proc_macro_error`), with the cycle in the module's types.
pub fn process_module_at(
    module: ItemMod,
    module_path: &Path,
    decycle: &Path,
    recurse_level: usize,
    support_infinite_cycle: bool,
) -> TokenStream {
    process_module_inner(
        module,
        Some(module_path),
        decycle,
        recurse_level,
        support_infinite_cycle,
    )
}

fn process_module_inner(
    mut module: ItemMod,
    module_path: Option<&Path>,
    decycle: &Path,
    recurse_level: usize,
    support_infinite_cycle: bool,
) -> TokenStream {
    // `module = path`: the cycle is solved on the module as written, where only a bare name is the
    // module's; the items re-emitted as written are qualified on the way out (`qualify`), after it.
    // See `crate::module_path`.
    let scope = module_path.map(|path| crate::module_path::ModuleScope::new(path, &module));
    let qualify = |item: &Item| match &scope {
        Some(scope) => scope.qualify_item(item),
        None => item.clone(),
    };
    // ... and the path is checked against the module, in the module (`ModuleScope::path_check`).
    let path_check = scope.as_ref().map(|scope| scope.path_check());
    // Value-level instantiation growth (a cyclic method reborrowing its own by-value generic
    // parameter) — neither engine can break it, and left alone it surfaces as a monomorphisation limit
    // naming generated internals. See `crate::growth`.
    //
    // Checked HERE, before the fold below strips the `#[decycle]` markers the obligation graph keys
    // on, but REPORTED after `set_dummy` installs the stripped module — otherwise the abort cascades
    // into a spurious "cannot find attribute `decycle`" from the unstripped fallback.
    let growth = crate::growth::check_value_generic_growth(&module, decycle);
    let contents = &mut module
        .content
        .as_mut()
        .unwrap_or_else(|| abort!(&module.semi, "needs content"))
        .1;
    // The decycle crate name as passed to the macro (leading segment of `decycle = …`, default
    // `decycle`) — used to match `#[<crate>::decycle]` on inner items without reading the manifest.
    let decycle_crate = &decycle.segments.first().unwrap().ident;
    let (traits, working_list, renames): (Vec<_>, Vec<_>, Vec<_>) = contents.iter_mut().fold(
        Default::default(),
        |(mut traits, mut working_list, mut renames), item| {
            match item {
                Item::Trait(ItemTrait { attrs, .. })
                | Item::TraitAlias(ItemTraitAlias { attrs, .. })
                | Item::Use(ItemUse { attrs, .. }) => {
                    // detect and remove #[decycle] attribute
                    let mut old_attrs = std::mem::take(attrs).into_iter();
                    let mut flag = false;
                    attrs.extend((&mut old_attrs).take_while(|attr| {
                        if crate::is_decycle_attribute(attr, decycle_crate) {
                            flag = true;
                        }
                        !flag
                    }));
                    attrs.extend(old_attrs);
                    if flag {
                        if let Item::Trait(item_trait) = item {
                            traits.push(item_trait.clone());
                        } else {
                            working_list.extend(process_trait_path(item));
                            renames.extend(collect_trait_renames(item));
                        }
                    }
                }
                _ => (),
            }
            (traits, working_list, renames)
        },
    );
    let all_traits: HashSet<Ident> = working_list
        .iter()
        .filter_map(|path| path.segments.last().map(|seg| seg.ident.clone()))
        .chain(traits.iter().map(|ItemTrait { ident, .. }| ident.clone()))
        .collect();
    proc_macro_error::set_dummy(quote! {
        #{&module.vis} #{&module.unsafety} mod #{&module.ident} {
            #(for content in contents.iter()) { #{qualify(content)} }
            #path_check
        }
    });
    // The fallback for an opt-out bound on a cycle (`crate::ranked::opt_out`), kept from the module
    // as written: without the global bounds of the routed traits, so that the impls do not overflow
    // on their own and the reported errors stand alone. Only `module = path` can report one, and
    // only on a module whose own attributes cannot remove it (a `#[cfg]` that is off compiles).
    let check_opt_outs =
        scope.is_some() && module.attrs.iter().all(crate::ranked::opt_out::is_inert);
    let quiet_dummy = check_opt_outs.then(|| {
        quote! {
            #{&module.vis} #{&module.unsafety} mod #{&module.ident} {
                #(for content in contents.iter()) {
                    #{crate::ranked::opt_out::quiet(qualify(content), &all_traits)}
                }
                #path_check
            }
        }
    });
    if let Err(e) = growth {
        abort!(e.span(), "{}", e);
    }
    for item in contents.iter() {
        match item {
            Item::Mod(item_mod) => {
                check_submodule(item_mod, decycle_crate);
            }
            // A `macro_rules!` DEFINITION is transparent: it introduces no item the rewrite has to
            // see, and it is re-emitted in place among the raw contents — ahead of the nested helper
            // modules that carry the adopted impls, so its textual scope still covers their bodies.
            // Any other item-position macro call may expand to impls or traits decycle can't see.
            Item::Macro(m) if m.mac.path.is_ident("macro_rules") => (),
            Item::Macro(_) => abort!(&item, "macro is not supported in #[decycle] module"),
            _ => (),
        }
    }
    check_no_deep_super_paths(contents);
    if traits.is_empty() && working_list.is_empty() {
        abort!(Span::call_site(), crate::NO_DECYCLE_TRAITS_MSG)
    }
    // Return-position `impl Trait` (RPITIT) in a #[decycle] trait method is an ACTUAL limitation of
    // the ranked re-entry engine: full-height re-entry needs a nameable fn-pointer return, but
    // Return-position `impl Trait` (RPITIT) in a `#[decycle]` trait method is unsupported by BOTH
    // engines and rejected up-front, in every mode, with an actionable message rather than a raw
    // solver error: the ranked re-entry fn-pointer type `fn(..) -> impl Trait` is not nameable
    // (E0562), and the structural layout cast can't reinterpret an opaque return type. An associated
    // type is the portable escape hatch — it names the return type, so both engines can carry it.
    for t in &traits {
        crate::finalize::check_no_impl_trait_output_in(t);
    }
    // `async fn` in a `#[decycle]` trait method is unsupported by the ranked engine (its return is an
    // opaque `impl Future`, which the rank rewrite can't thread through — the raw symptom is a
    // confusing `E0308` blaming the attribute). Reject it up-front, in BOTH modes, with an actionable
    // message. (The structural engine rejects it too — there it was outright unsound.)
    for item in contents.iter() {
        if let Item::Impl(im) = item {
            for it in &im.items {
                if let ImplItem::Fn(f) = it {
                    if let Some(a) = &f.sig.asyncness {
                        abort!(
                            a,
                            "decycle: `async fn` in method `{}` of a #[decycle] cycle is not supported — its `impl Future` return can't be threaded through the rank rewrite", f.sig.ident;
                            help = "return a boxed future (`-> Pin<Box<dyn Future<Output = ..>>>`) instead"
                        );
                    }
                }
            }
        }
    }
    // `#[track_caller]` on a method in an UNBOUNDED cycle is a limitation of the ranked re-entry
    // engine: full-height re-entry dispatches through a transmuted fn pointer, which cannot carry the
    // implicit caller-location argument, so `Location::caller()` reports the wrong site once recursion
    // passes the floor. It IS correct for shallow calls, but that can't be told apart at compile time,
    // and a silently-wrong location is worse than a clear rejection.
    if support_infinite_cycle {
        for item in contents.iter() {
            if let Item::Impl(im) = item {
                for it in &im.items {
                    if let ImplItem::Fn(f) = it {
                        if let Some(a) = f.attrs.iter().find(|a| a.path().is_ident("track_caller")) {
                            abort!(
                                a,
                                "decycle: `#[track_caller]` on method `{}` in an unbounded #[decycle] cycle is not supported — the re-entry fn-pointer indirection loses the caller location past the recursion floor", f.sig.ident;
                                help = "use `#[decycle(structural)]`, or set `support_infinite_cycle = false`"
                            );
                        }
                    }
                }
            }
        }
    }
    // `impl Tr for self::Term` is `impl Tr for Term`: read (and re-emit) a self type in the bare
    // spelling, which is the one everything below matches a cycle member by.
    for item in contents.iter_mut() {
        if let Item::Impl(im) = item {
            if impls_routed_trait(im, &all_traits) {
                bare_self_type(im);
            }
        }
    }
    // The impls the engine takes over: those of a routed trait spelled bare (see the split below),
    // for a self type the module names by its bare name. A self type spelled by any other path
    // (`impl Tr for crate::m::Term`, `impl Tr for crate::other::B`) names the item itself, the
    // opt-out, exactly as a path-spelled trait does: the impl takes no part in the cycle and is
    // re-emitted as written.
    let adopts = |im: &ItemImpl| {
        impls_routed_trait(im, &all_traits) && !self_ty_spelled_by_path(&im.self_ty)
    };
    // Head idents of the types that IMPLEMENT a cyclic trait here — the only heads a bare cyclic
    // where-bound may target (besides `Self` / an impl type-param). A bound whose head is a foreign
    // container (`Box<Stmt>: Tr`) has no ranked impl to descend through, so it's flagged below.
    let cycle_self_heads: HashSet<Ident> = contents
        .iter()
        .filter_map(|item| {
            let Item::Impl(im) = item else { return None };
            adopts(im).then(|| type_head_ident(&im.self_ty)).flatten()
        })
        .collect();
    // Peel wrapped cyclic bounds (`Vec<Box<Stmt>>: Tr` ⇒ `Stmt: Tr`) BEFORE validating, so the
    // validation below only rejects heads that genuinely cannot be made rankable.
    for item in contents.iter_mut() {
        if let Item::Impl(im) = item {
            let impl_type_params = crate::names::type_param_idents(&im.generics);
            crate::ranked::peel::peel_cyclic_bounds(im, &all_traits, &cycle_self_heads, &|ty| {
                is_local_impl_bound_target(ty, &impl_type_params)
            });
        }
    }
    for item in contents.iter() {
        if let Item::Impl(item_impl) = item {
            validate_impl_where_bounds(item_impl, &all_traits, &cycle_self_heads);
        }
    }
    // A bound naming an own type by its path (the opt-out, left as written, as peeling left it) on
    // a cycle overflows in rustc, on the generated `…Ranked` traits. Say so in the module's terms,
    // at each such bound, after the fallback is made quiet. See `crate::ranked::opt_out`.
    if let (Some(scope), Some(quiet_dummy)) = (&scope, quiet_dummy) {
        let cycles =
            crate::ranked::opt_out::opt_out_cycles(contents, scope, &all_traits, recurse_level);
        if !cycles.is_empty() {
            proc_macro_error::set_dummy(quiet_dummy);
            for cycle in cycles {
                emit_error!(cycle.at, "{}", cycle.message; help = "{}", cycle.help);
            }
            abort_if_dirty();
        }
    }
    let (raw_contents, contents): (Vec<_>, Vec<_>) = contents
        .iter()
        .map(|content| {
            if let Item::Impl(
                item_impl @ ItemImpl {
                    trait_: Some(_), ..
                },
            ) = content
            {
                // Taken over when the (self::-normalized) trait path is a single segment naming a
                // routed trait, for a self type that is no path-spelled opt-out (`adopts`).
                // NOTE: a still-qualified trait path (`impl crate::foo::MyTrait for X`,
                // `impl super::MyTrait for X`) is intentionally left as an ordinary,
                // non-decycled impl rather than flagged — the same qualified-reference
                // form is the established way to give a FOREIGN/non-cyclic type an
                // impl of the ORIGINAL, un-ranked trait from inside a #[decycle] module
                // (mirrored by the identical, deliberate pattern in where-bounds; see
                // `validate_impl_where_bounds`), so it can't be reliably distinguished
                // from a genuine mis-qualification at this syntactic level. A path-spelled
                // self type (`impl MyTrait for crate::m::X`) is the same opt-out.
                if adopts(item_impl) {
                    let mut normalized_impl = item_impl.clone();
                    let trait_path = &mut normalized_impl.trait_.as_mut().unwrap().1;
                    crate::names::strip_leading_self(trait_path);
                    return (None, Some(normalized_impl));
                }
            }
            (Some(content), None)
        })
        .fold(
            Default::default(),
            |(mut raw_contents, mut contents), (raw_content, content)| {
                raw_contents.extend(raw_content);
                contents.extend(content);
                (raw_contents, contents)
            },
        );
    // Neutralise the hazards our own nesting creates (glob ambiguity, shifted relative paths)
    // before `finalize` re-emits these impls two modules deep — see `crate::nesting`.
    let mut contents = contents;
    // The impls of one cyclic trait must be able to prove each other's leaf premises — but only the
    // ones whose rank chains can actually REACH each other. The group used to be "every adopted impl
    // of the trait", so a standalone acyclic impl silently inherited a cycle's premises (`C<T>`
    // acquiring `T: Clone` rejects code that is valid without the macro), and two disjoint cycles of
    // one trait unioned each other's. Scope each impl's inheritance to the impls its own cyclic
    // bounds (transitively) obligate instead — see `sharing_sources` and `crate::ranked::sharing`.
    // The heads `finalize` reads a bare bound as an edge on (`remove_cyclic_bounds`): those of the
    // impls taken over. A bare bound on any other type is a premise, shared like one.
    let adopted_heads: HashSet<Ident> = contents
        .iter()
        .filter_map(|im| type_head_ident(&im.self_ty))
        .collect();
    let is_edge = |im: &ItemImpl, pred: &WherePredicate| {
        crate::ranked::sharing::is_ranked_edge(im, pred, &all_traits, &adopted_heads)
    };
    // The same premises, carried along every cyclic bound, whatever its trait, with the reached
    // impl's parameters mapped through the bound (`crate::premise_flow`). The by-name union below
    // is per trait, so in a cycle alternating between traits (`Expr: Eval` needing `Term: Size`) a
    // member never received the premises of the impls of the other traits it reaches. Its re-entry
    // registrations then failed `reachable_side_bounds_ok` and were skipped, and a recursion past the
    // floor panicked ("re-entry fn not registered"). Computed on the impls as written, before the
    // by-name union adds to them.
    let along_bounds = {
        let impls: Vec<&ItemImpl> = contents.iter().collect();
        let edges: Vec<Vec<crate::premise_flow::CycleEdge>> = impls
            .iter()
            .map(|im| ranked_cycle_edges(im, &impls, &all_traits, &adopted_heads, &is_edge))
            .collect();
        crate::premise_flow::mapped_premises(
            &impls,
            &edges,
            &is_edge,
            crate::premise_flow::Respell::TypeParams,
        )
    };
    for trait_ident in &all_traits {
        let is_member = |im: &ItemImpl| {
            im.trait_.as_ref().is_some_and(|(_, path, _)| {
                path.leading_colon.is_none()
                    && path.segments.len() == 1
                    && path.segments[0].ident == *trait_ident
            })
        };
        let members: Vec<&ItemImpl> = contents.iter().filter(|im| is_member(im)).collect();
        if members.len() < 2 {
            continue;
        }
        let sources = sharing_sources(&members, trait_ident);
        let mut group: Vec<&mut ItemImpl> = contents.iter_mut().filter(|im| is_member(im)).collect();
        crate::ranked::sharing::share_side_predicates_scoped(&mut group, &is_edge, &sources);
    }
    for (im, add) in contents.iter_mut().zip(along_bounds) {
        crate::premise_flow::add_premises(im, add);
    }

    // Restricted to types this module DEFINES: a foreign self type (`impl Tr for &str`) is reachable
    // by one path only, so it is neither ambiguous nor bindable as `use self::…`.
    let local_types: HashSet<Ident> = raw_contents
        .iter()
        .filter_map(|item| match item {
            Item::Struct(it) => Some(it.ident.clone()),
            Item::Enum(it) => Some(it.ident.clone()),
            Item::Union(it) => Some(it.ident.clone()),
            Item::Type(it) => Some(it.ident.clone()),
            _ => None,
        })
        .collect();
    let local_cycle_heads: HashSet<Ident> =
        cycle_self_heads.intersection(&local_types).cloned().collect();
    let alias_items =
        crate::ranked::nesting::defuse_nesting(
        &mut contents,
        &local_cycle_heads,
        &all_traits,
        &module.ident,
    );

    let first_path = working_list.first().cloned();
    let mut args = crate::finalize::FinalizeArgs {
        working_list,
        traits,
        contents,
        recurse_level,
        support_infinite_cycle,
        renames,
        // C2: this path keeps the working-list convention (only a direct, programmatic
        // caller of `finalize` sets `also_rank`).
        also_rank: Vec::new(),
        // D1: this path keeps the working-list convention (only a direct, programmatic
        // caller of `finalize` sets `decycle_path`).
        decycle_path: None,
    };
    args.working_list.push(parse_quote!(#decycle::__finalize));
    quote! {
        #(for attr in &module.attrs) { #attr }
        #{&module.vis} #{&module.unsafety} #{&module.mod_token} #{&module.ident} {

            #(for alias_item in &alias_items) { #alias_item }

            #(for raw_content in raw_contents) { #{qualify(raw_content)} }

            #path_check

            #(if first_path.is_some()) {
                #{ args.cont_list().invoke(args.rest_tokens()) }
            }
            #(else) {
                #{ crate::finalize::finalize(args) }
            }
        }
    }
}
