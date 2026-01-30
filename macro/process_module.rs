use proc_macro::Span;
use proc_macro2::TokenStream;
use proc_macro_error::*;
use std::collections::HashSet;
use syn::*;
use template_quote::quote;

fn check_submodule(module: &ItemMod) {
    use syn::visit::Visit;
    struct Visitor;
    impl<'ast> Visit<'ast> for Visitor {
        fn visit_attribute(&mut self, i: &'ast syn::Attribute) {
            if super::is_decycle_attribute(i) {
                abort!(&i, "#[decycle] is not supported in nested modules")
            }
            syn::visit::visit_attribute(self, i);
        }
    }
    Visitor.visit_item_mod(module);
}

pub fn process_trait_path(item: &Item) -> Vec<Path> {
    match item {
        Item::Trait(ItemTrait { ident, .. }) | Item::TraitAlias(ItemTraitAlias { ident, .. }) => {
            vec![super::ident_to_path(ident)]
        }
        Item::Use(ItemUse { tree, .. }) => {
            fn process_use_tree(tree: &UseTree) -> Vec<Path> {
                match tree {
                    UseTree::Path(UsePath { tree, .. }) => process_use_tree(&tree),
                    UseTree::Name(UseName { ident })
                    | UseTree::Rename(UseRename { rename: ident, .. }) => {
                        vec![super::ident_to_path(ident)]
                    }
                    UseTree::Glob(use_glob) => {
                        abort!(use_glob, "glob is not supported in #[decycle] use")
                    }
                    UseTree::Group(UseGroup { items, .. }) => {
                        items.iter().map(process_use_tree).flatten().collect()
                    }
                }
            }
            process_use_tree(tree)
        }
        _ => unreachable!(),
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
                        if super::is_decycle_attribute(&attr) {
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
        quote! { #{&module.ident} {#(for content in contents.clone()) { #content }} },
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
