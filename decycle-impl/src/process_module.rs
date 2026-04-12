use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro_error::*;
use std::collections::HashSet;
use syn::spanned::Spanned;
use syn::*;
use template_quote::quote;

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
        Item::Trait(ItemTrait { ident, .. }) | Item::TraitAlias(ItemTraitAlias { ident, .. }) => {
            vec![crate::ident_to_path(ident)]
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

fn collect_local_type_idents(contents: &[Item]) -> Vec<Ident> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in contents {
        let ident = match item {
            Item::Struct(ItemStruct { ident, .. })
            | Item::Enum(ItemEnum { ident, .. })
            | Item::Union(ItemUnion { ident, .. })
            | Item::Type(ItemType { ident, .. }) => ident,
            _ => continue,
        };
        if seen.insert(ident.to_string()) {
            out.push(ident.clone());
        }
    }
    out
}

fn local_types_help_message(local_type_idents: &[Ident]) -> String {
    if local_type_idents.is_empty() {
        return "use `Self` or an internal type defined within the `#[decycle]` module".to_owned();
    }
    let types = local_type_idents
        .iter()
        .map(|ident| format!("`{ident}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "use `Self` or an internal type defined within the `#[decycle]` module, such as {types}"
    )
}

fn validate_impl_where_bounds(
    item_impl: &ItemImpl,
    all_traits: &HashSet<Ident>,
    local_type_idents: &[Ident],
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
            let Some(last_segment) = path.segments.last() else {
                continue;
            };
            if all_traits.contains(&last_segment.ident) && has_assoc_constraints(path) {
                let help_message = local_types_help_message(local_type_idents);
                abort!(
                    path,
                    "unsupported #[decycle] bound with associated constraints on non-local type";
                    help = bounded_ty.span() => "this `where`-clause bound in the `impl` block targets a non-local type";
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
    let (traits, working_list): (Vec<_>, Vec<_>) = contents.iter_mut().fold(
        Default::default(),
        |(mut traits, mut working_list), item| {
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
                        }
                    }
                }
                _ => (),
            }
            (traits, working_list)
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
    if traits.is_empty() && working_list.is_empty() {
        abort!(
            Span::call_site(),
            "cannot detect traits nor `use` statement annotated witn #[decycle]"
        )
    }
    let all_traits: HashSet<Ident> = working_list
        .iter()
        .filter_map(|path| path.segments.last().map(|seg| seg.ident.clone()))
        .chain(traits.iter().map(|ItemTrait { ident, .. }| ident.clone()))
        .collect();
    let local_type_idents = collect_local_type_idents(contents);
    for item in contents.iter() {
        if let Item::Impl(item_impl) = item {
            validate_impl_where_bounds(item_impl, &all_traits, &local_type_idents);
        }
    }
    let (raw_contents, contents): (Vec<_>, Vec<_>) = contents
        .iter()
        .map(|content| {
            if let Item::Impl(
                item_impl @ ItemImpl {
                    trait_: Some((_, trait_path, _)),
                    ..
                },
            ) = content
            {
                // Check the trait_path contains just one segment
                if trait_path.segments.len() == 1 {
                    if let Some(seg) = trait_path.segments.first() {
                        if all_traits.contains(&seg.ident) {
                            // The item is impl of a trait annotated with #[decycle]
                            return (None, Some(item_impl.clone()));
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
