use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro_error::*;
use std::collections::HashSet;
use syn::spanned::Spanned;
use syn::*;
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

fn is_local_impl_bound_target(ty: &Type, impl_type_params: &HashSet<Ident>) -> bool {
    let Type::Path(TypePath { qself: None, path }) = ty else {
        return false;
    };
    if path.segments.len() != 1 {
        return false;
    }
    let ident = &path.segments[0].ident;
    ident == "Self" || impl_type_params.contains(ident)
}

/// The head (outermost path) ident of a type: `Box` for `Box<Stmt>`, `B` for `B<T>`, `Stmt` for
/// `Stmt`. `None` for a non-path type (`&Stmt`, `(A, B)`) or a `<T as Tr>::X` qself.
fn type_head_ident(ty: &Type) -> Option<Ident> {
    match ty {
        Type::Path(TypePath { qself: None, path }) => path.segments.last().map(|s| s.ident.clone()),
        _ => None,
    }
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
    let impl_type_params: HashSet<Ident> = item_impl
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Type(ty) => Some(ty.ident.clone()),
            _ => None,
        })
        .collect();

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
            crate::helper::strip_leading_self(&mut path);
            // Single-segment match only (after self::-normalization) — a multi-segment
            // path (`some::mod::Foo`) merely sharing a last segment with a #[decycle]
            // trait is a DIFFERENT item; matching on the last segment alone was a false
            // positive. NOTE: a still-multi-segment, qualified reference to a #[decycle]
            // trait (`super::Foo`, `crate::mod::Foo`) is intentionally NOT flagged here —
            // it's the established, working way to bind a FOREIGN (non-cyclic) type to
            // the ORIGINAL, un-ranked trait in a side-bound (`Foreign: super::Foo`); only
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
                let head_ok =
                    type_head_ident(bounded_ty).is_some_and(|h| cycle_self_heads.contains(&h));
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
                // A foreign or container head like `Box<Stmt>` has no ranked impl to descend
                // through, so rustc would otherwise emit a raft of raw `Box<Stmt>: TrRanked<…>`
                // overflow errors at the useless module span. Reject up-front with a legible
                // message on the user's own bound instead (structural forwards such a bound
                // through a blanket `impl<T: Tr> Tr for Box<T>`; ranked's rank chain can't).
                if !head_ok {
                    abort!(
                        bounded_ty,
                        "decycle: this cyclic bound's target is not a type the ranked engine can rank-lower — its head is a container or non-cycle type (e.g. `Box<Stmt>`), so the ranked chain has no floor impl to descend through";
                        help = "use `#[decycle(structural)]` (it forwards a wrapped bound via a blanket `impl<T: Tr> Tr for Box<T>`), or bound a bare cycle type instead"
                    );
                }
            }
        }
    }
}

/// Apply the ranked `#[decycle]` transformation to `module`, programmatically.
///
/// `decycle` is the path to the decycle crate (its leading segment names the crate, used to
/// recognise `#[<crate>::decycle]` on inner items). `recurse_level` sets the compile-time expansion
/// depth; `support_infinite_cycle` toggles the runtime re-entry registry (unbounded depth) versus a
/// fixed-depth floor. For macro authors wrapping `#[decycle]`; most users should use the attribute.
pub fn process_module(
    mut module: ItemMod,
    decycle: &Path,
    recurse_level: usize,
    support_infinite_cycle: bool,
) -> TokenStream {
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
    proc_macro_error::set_dummy(
        quote! {
            #{&module.vis} #{&module.unsafety} mod #{&module.ident} {
                #(for content in contents.clone()) { #content }
            }
        },
    );
    for item in contents.iter() {
        match item {
            Item::Mod(item_mod) => {
                check_submodule(item_mod, decycle_crate);
            }
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
        for item in &t.items {
            if let TraitItem::Fn(tf) = item {
                if crate::finalize::sig_has_impl_trait_output(&tf.sig) {
                    abort!(
                        &tf.sig.output,
                        "decycle: return-position `impl Trait` in method `{}` of #[decycle] trait `{}` is not supported", tf.sig.ident, t.ident;
                        help = "declare an associated type on the trait and return it instead (e.g. `type Output; fn m(&self) -> Self::Output`)"
                    );
                }
            }
        }
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
    let all_traits: HashSet<Ident> = working_list
        .iter()
        .filter_map(|path| path.segments.last().map(|seg| seg.ident.clone()))
        .chain(traits.iter().map(|ItemTrait { ident, .. }| ident.clone()))
        .collect();
    // Head idents of the types that IMPLEMENT a cyclic trait here — the only heads a bare cyclic
    // where-bound may target (besides `Self` / an impl type-param). A bound whose head is a foreign
    // container (`Box<Stmt>: Tr`) has no ranked impl to descend through, so it's flagged below.
    let cycle_self_heads: HashSet<Ident> = contents
        .iter()
        .filter_map(|item| {
            let Item::Impl(im) = item else { return None };
            let (_, trait_path, _) = im.trait_.as_ref()?;
            let mut tp = trait_path.clone();
            crate::helper::strip_leading_self(&mut tp);
            (tp.segments.len() == 1 && all_traits.contains(&tp.segments[0].ident))
                .then(|| type_head_ident(&im.self_ty))
                .flatten()
        })
        .collect();
    for item in contents.iter() {
        if let Item::Impl(item_impl) = item {
            validate_impl_where_bounds(item_impl, &all_traits, &cycle_self_heads);
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
                let mut normalized_impl = item_impl.clone();
                let trait_path = &mut normalized_impl.trait_.as_mut().unwrap().1;
                crate::helper::strip_leading_self(trait_path);
                // Check the (self::-normalized) trait_path contains just one segment.
                // NOTE: a still-qualified trait path (`impl crate::foo::MyTrait for X`,
                // `impl super::MyTrait for X`) is intentionally left as an ordinary,
                // non-decycled impl rather than flagged — the same qualified-reference
                // form is the established way to give a FOREIGN/non-cyclic type an
                // impl of the ORIGINAL, un-ranked trait from inside a #[decycle] module
                // (mirrored by the identical, deliberate pattern in where-bounds; see
                // `validate_impl_where_bounds`), so it can't be reliably distinguished
                // from a genuine mis-qualification at this syntactic level.
                if trait_path.segments.len() == 1 {
                    if let Some(seg) = trait_path.segments.first() {
                        if all_traits.contains(&seg.ident) {
                            // The item is impl of a trait annotated with #[decycle]
                            return (None, Some(normalized_impl));
                        }
                    }
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

            #(for raw_content in raw_contents) { #raw_content }

            #(if let Some(first_path) = first_path) {
                #first_path! { #args }
            }
            #(else) {
                #{ crate::finalize::finalize(args) }
            }
        }
    }
}
