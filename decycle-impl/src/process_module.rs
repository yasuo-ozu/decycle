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

fn check_submodule(module: &ItemMod) {
    use syn::visit::Visit;
    struct Visitor;
    impl<'ast> Visit<'ast> for Visitor {
        fn visit_attribute(&mut self, i: &'ast syn::Attribute) {
            if crate::is_decycle_attribute(i) {
                abort!(&i, "#[decycle] is not supported in nested modules")
            }
            syn::visit::visit_attribute(self, i);
        }
    }
    Visitor.visit_item_mod(module);
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

fn validate_impl_where_bounds(item_impl: &ItemImpl, all_traits: &HashSet<Ident>) {
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
            if all_traits.contains(&last_segment.ident) && has_assoc_constraints(&path) {
                // F6: the earlier wording ("...on non-local type") described what the check
                // rejects, but the check doesn't actually key on locality — a bound on a
                // module-LOCAL struct/enum (anything other than `Self` or one of this impl's
                // own type parameters) is rejected exactly the same way. State what's
                // ACCEPTED instead, which is unambiguous either way.
                let help_message = local_types_help_message(&impl_type_params);
                abort!(
                    path,
                    "associated-type constraints in #[decycle] impl where-clauses are only supported on `Self` or the impl's own type parameters";
                    help = bounded_ty.span() => "{}", help_message
                );
            }
        }
    }
}

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
                        if crate::is_decycle_attribute(attr) {
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
                check_submodule(item_mod);
            }
            Item::Macro(_) => abort!(&item, "macro is not supported in #[decycle] module"),
            _ => (),
        }
    }
    check_no_deep_super_paths(contents);
    if traits.is_empty() && working_list.is_empty() {
        abort!(
            Span::call_site(),
            "cannot detect traits nor `use` statement annotated with #[decycle]"
        )
    }
    let all_traits: HashSet<Ident> = working_list
        .iter()
        .filter_map(|path| path.segments.last().map(|seg| seg.ident.clone()))
        .chain(traits.iter().map(|ItemTrait { ident, .. }| ident.clone()))
        .collect();
    for item in contents.iter() {
        if let Item::Impl(item_impl) = item {
            validate_impl_where_bounds(item_impl, &all_traits);
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
