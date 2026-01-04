use proc_macro::Span;
use proc_macro2::TokenStream;
use proc_macro_error::*;
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

pub fn process_module(mut module: ItemMod, decycle: &Path) -> TokenStream {
    let contents = &mut module
        .content
        .as_mut()
        .unwrap_or_else(|| abort!(&module.semi, "needs content"))
        .1;
    // preprocess. collect annotated paths
    let trait_paths: Vec<(Path, Option<ItemTrait>)> = contents
        .iter_mut()
        .filter_map(|content| match content {
            Item::Mod(item_mod) => {
                check_submodule(item_mod);
                None
            }
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
                let tr = if let Item::Trait(item_trait) = content {
                    Some(item_trait.clone())
                } else {
                    None
                };
                if flag {
                    Some(
                        process_trait_path(content)
                            .into_iter()
                            .map(|path| (path, tr.clone()))
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                }
            }
            Item::Macro(_) => abort!(&content, "macro is not supported in #[decycle] module"),
            _ => None,
        })
        .flatten()
        .collect();
    if trait_paths.is_empty() {
        abort!(
            Span::call_site(),
            "cannot detect traits annotated witn #[decycle]"
        )
    }
    let mut working_list = trait_paths
        .iter()
        .filter(|(_, opt)| opt.is_none())
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    let first_path = working_list.pop();
    let mut args = crate::finalize::FinalizeArgs {
        working_list,
        traits: trait_paths
            .iter()
            .cloned()
            .filter_map(|(_, opt)| opt)
            .collect(),
        contents: contents
            .iter()
            .filter_map(|content| {
                if let Item::Impl(item) = content.clone() {
                    Some(item)
                } else {
                    None
                }
            })
            .collect(),
    };
    if let Some(first_path) = first_path {
        // call trait macros recursively
        args.working_list.push(parse_quote!(#decycle::__finalize));
        quote! {
            #(for attr in &module.attrs) { #attr }
            #{&module.vis} #{&module.unsafety} #{&module.mod_token} #{&module.ident} {
                #(for content in contents.iter().filter(|content| !matches!(content, Item::Impl(_)))) { #content }

                #first_path! { #args }
            }
        }
    } else {
        // call finalize
        crate::finalize::finalize(args)
    }
}
