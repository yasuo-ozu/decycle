//! Learning what a module's `#[decycle] use` items carry, before the module is expanded.
//!
//! A module's `impl Tr for X` is matched to a `#[decycle]` trait by NAME, so the expansion has to
//! know the names of the traits behind the `#[decycle] use` items. Usually the `use` says it:
//! `#[decycle] use path::Tr;` carries `Tr`, and `use path::Tr as Local;` carries what the module
//! calls `Local`. A trait announced with `macro_name = N` is reached under a trampoline name that
//! is not its own, though, and `#[decycle] use path::N;` does not say which trait it carries; the
//! module's plain `use path::Tr;` names it, but a proc macro cannot tell that this `use` and the
//! `#[decycle] use` belong together. Only the trampoline knows, so in that one situation the
//! trampolines are called first, for the names alone:
//!
//! ```text
//! mod m {                                  mod m {
//!     use path::Tr;                            #[allow(unused_imports)] use path::N;
//!     #[decycle] use path::N;       ==>        N! { [N!, decycle::__discover!] .. the rest of the items .. }
//!     impl Tr for X where Y: Tr {..}       }
//! }
//! ```
//!
//! `__discover!` ([`finish`]) is called with the trait of each trampoline, and expands the rest of
//! the module as `#[decycle]` would have, knowing their names. It expands inside the module, where
//! the `use` items live, and so emits the module's body and nothing around it.
//!
//! This is needed only when it is needed: when a `#[decycle] use` is not renamed with `as` and an
//! `impl` in the module names a trait that nothing in the module accounts for but a plain `use`
//! ([`start`]). A module that does not match is expanded at once, as before, with no trampoline
//! called for the names.

use proc_macro2::{Span, TokenStream};
use proc_macro_error::*;
use std::collections::HashSet;
use syn::parse::{Parse, ParseStream};
use syn::*;
use template_quote::quote;

/// What the trampolines told a module's expansion (see the module documentation).
pub(crate) struct Discovered {
    /// The names the module spells the traits its `#[decycle] use` items carry by.
    pub carried: Vec<Ident>,
    /// The module's `#[decycle] use` items, `#[decycle]` markers intact. They were emitted ahead of
    /// the trampolines, so the expansion does not re-emit them.
    pub uses: Vec<ItemUse>,
}

/// The engine a module is expanded with, for [`start`] and [`finish`].
#[derive(Clone, Copy)]
pub enum Engine {
    /// `#[decycle(recurse_level = .., support_infinite_cycle = ..)]`.
    Ranked {
        recurse_level: usize,
        support_infinite_cycle: bool,
    },
    /// `#[decycle(structural)]`.
    Structural,
}

/// The names a `use` tree binds, each with whether it was renamed with an `as`. A glob binds none.
fn bindings(tree: &UseTree, out: &mut Vec<(Ident, bool)>) {
    match tree {
        UseTree::Path(p) => bindings(&p.tree, out),
        // `use a::b::{self}` binds `b`, which this does not know; it is not a trait name worth
        // looking for.
        UseTree::Name(n) if n.ident != "self" => out.push((n.ident.clone(), false)),
        UseTree::Rename(r) if r.rename != "_" => out.push((r.rename.clone(), true)),
        UseTree::Group(g) => g.items.iter().for_each(|t| bindings(t, out)),
        UseTree::Name(_) | UseTree::Rename(_) | UseTree::Glob(_) => (),
    }
}

/// The first segment of every path a `use` tree imports from (`a` of `use a::b::{c, d::e}`).
fn first_segments(tree: &UseTree, out: &mut Vec<Ident>) {
    match tree {
        UseTree::Path(p) => out.push(p.ident.clone()),
        UseTree::Name(n) => out.push(n.ident.clone()),
        UseTree::Rename(r) => out.push(r.ident.clone()),
        UseTree::Glob(_) => (),
        UseTree::Group(g) => g.items.iter().for_each(|t| first_segments(t, out)),
    }
}

/// The names the items of a module define: what a path inside the module may start at.
fn defined_names(items: &[Item], out: &mut HashSet<Ident>) {
    for item in items {
        let ident = match item {
            Item::Mod(i) => &i.ident,
            Item::Struct(i) => &i.ident,
            Item::Enum(i) => &i.ident,
            Item::Union(i) => &i.ident,
            Item::Type(i) => &i.ident,
            Item::Trait(i) => &i.ident,
            Item::TraitAlias(i) => &i.ident,
            Item::Fn(i) => &i.sig.ident,
            Item::Const(i) => &i.ident,
            Item::Static(i) => &i.ident,
            Item::Macro(ItemMacro {
                ident: Some(ident), ..
            }) => ident,
            Item::Use(u) => {
                let mut leaves = Vec::new();
                bindings(&u.tree, &mut leaves);
                out.extend(leaves.into_iter().map(|(ident, _)| ident));
                continue;
            }
            _ => continue,
        };
        out.insert(ident.clone());
    }
}

/// If `module` needs the trampolines called for the names of its traits before it can be expanded,
/// the expansion that calls them (`__discover!` then expands the rest, see [`finish`]); `None` if it
/// can be expanded at once.
///
/// It needs them when a `#[decycle] use` is not renamed (so that its name need not be the trait's)
/// and the module has an `impl Tr for ..` that no `#[decycle]` item accounts for but a plain
/// `use ..::Tr;` does: the shape of `use path::Tr; #[decycle] use path::N;`.
///
/// The trampolines are called from a module that holds the `#[decycle] use` items and nothing else
/// yet, so a `#[decycle] use` that starts at an item of the module itself (`self::inner::N`, or `N`
/// through a `use` of its own) cannot resolve there. Such a module is not asked for the names:
/// `crate::`, `super::`, `::krate::` and the name of another crate are the paths that work.
pub fn start(module: &ItemMod, decycle: &Path, engine: Engine) -> Option<TokenStream> {
    let decycle_crate = &decycle.segments.first()?.ident;
    let items = &module.content.as_ref()?.1;
    let marked = |attrs: &[Attribute]| {
        attrs
            .iter()
            .any(|a| crate::names_decycle_attribute(a, decycle_crate))
    };
    // The names the module's `#[decycle]` items bind (traits, and the local names of the `use`s).
    let mut routed: HashSet<Ident> = HashSet::new();
    let mut unrenamed = false;
    // The names its plain `use` items bind, and the traits its impls implement.
    let mut imported: Vec<(Ident, bool)> = Vec::new();
    let mut implemented: HashSet<Ident> = HashSet::new();
    let (mut uses, mut rest) = (Vec::new(), Vec::new());
    for item in items {
        match item {
            Item::Use(u) if marked(&u.attrs) => {
                let mut leaves = Vec::new();
                bindings(&u.tree, &mut leaves);
                for (local, renamed) in leaves {
                    unrenamed |= !renamed;
                    routed.insert(local);
                }
                uses.push(u.clone());
                continue;
            }
            Item::Use(u) => bindings(&u.tree, &mut imported),
            Item::Trait(t) if marked(&t.attrs) => {
                routed.insert(t.ident.clone());
            }
            Item::Impl(ItemImpl {
                trait_: Some((_, path, _)),
                ..
            }) => {
                let mut path = path.clone();
                crate::helper::strip_leading_self(&mut path);
                if path.segments.len() == 1 {
                    implemented.insert(path.segments[0].ident.clone());
                }
            }
            _ => (),
        }
        rest.push(item.clone());
    }
    let imported: HashSet<Ident> = imported.into_iter().map(|(ident, _)| ident).collect();
    let mut defined = HashSet::new();
    defined_names(&rest, &mut defined);
    let mut starts = Vec::new();
    for u in &uses {
        if u.leading_colon.is_none() {
            first_segments(&u.tree, &mut starts);
        }
    }
    if starts
        .iter()
        .any(|ident| ident == "self" || defined.contains(ident))
    {
        return None;
    }
    if !unrenamed
        || !implemented
            .iter()
            .any(|tr| !routed.contains(tr) && imported.contains(tr))
    {
        return None;
    }

    // The `#[decycle] use` items go first, as the module has them, so that the trampolines resolve
    // from inside it; the engine does not emit them again.
    let structural = matches!(engine, Engine::Structural);
    let emitted: Vec<ItemUse> = uses
        .iter()
        .cloned()
        .map(|mut u| {
            if let Some(ix) = u
                .attrs
                .iter()
                .position(|a| crate::names_decycle_attribute(a, decycle_crate))
            {
                u.attrs.remove(ix);
            }
            if structural {
                u.attrs.push(parse_quote!(#[allow(unused_imports)]));
            }
            u
        })
        .collect();
    let list: announce::helper::ContList = uses
        .iter()
        .flat_map(|u| {
            let mut leaves = Vec::new();
            bindings(&u.tree, &mut leaves);
            leaves
        })
        .map(|(local, _)| announce::helper::Cont::bang(crate::ident_to_path(&local)))
        .chain(std::iter::once(announce::helper::Cont::bang(
            parse_quote!(#decycle::__discover),
        )))
        .collect();
    let identity = LitStr::new(&crate::get_crate_identity(), Span::call_site());
    let version = env!("CARGO_PKG_VERSION");
    let engine = match engine {
        Engine::Ranked {
            recurse_level,
            support_infinite_cycle,
        } => quote! { ranked #recurse_level #support_infinite_cycle },
        Engine::Structural => quote! { structural },
    };
    let ident = &module.ident;
    let call = list.invoke(quote! {
        #identity #version ( #decycle ) #engine #ident
        { #(#uses)* }
        { #(#rest)* }
    });
    // Inner attributes (`#![allow(..)]`) lead the braces, ahead of everything.
    Some(quote! {
        #(for attr in module.attrs.iter().filter(|a| matches!(a.style, AttrStyle::Outer))) { #attr }
        #{&module.vis} #{&module.unsafety} #{&module.mod_token} #ident {
            #(for attr in module.attrs.iter().filter(|a| matches!(a.style, AttrStyle::Inner(_)))) { #attr }
            #(#emitted)*
            #call
        }
    })
}

/// The input of `__discover!`: the REST that [`start`] wrote after the trampolines' entries.
struct Rest {
    decycle: Path,
    engine: Engine,
    ident: Ident,
    uses: Vec<ItemUse>,
    items: Vec<Item>,
}

impl Parse for Rest {
    fn parse(input: ParseStream) -> Result<Self> {
        let _identity: LitStr = input.parse()?;
        let version: LitStr = input.parse()?;
        let expected = env!("CARGO_PKG_VERSION");
        if version.value() != expected {
            abort!(
                Span::call_site(),
                "version mismatch: expected '{}', got '{}'",
                expected,
                version.value()
            )
        }
        let decycle;
        parenthesized!(decycle in input);
        let decycle: Path = decycle.parse()?;
        let kind: Ident = input.parse()?;
        let engine = if kind == "structural" {
            Engine::Structural
        } else {
            let level: LitInt = input.parse()?;
            let infinite: LitBool = input.parse()?;
            Engine::Ranked {
                recurse_level: level.base10_parse()?,
                support_infinite_cycle: infinite.value,
            }
        };
        let ident: Ident = input.parse()?;
        let uses_content;
        braced!(uses_content in input);
        let mut uses = Vec::new();
        while !uses_content.is_empty() {
            uses.push(uses_content.parse()?);
        }
        let items_content;
        braced!(items_content in input);
        let mut items = Vec::new();
        while !items_content.is_empty() {
            items.push(items_content.parse()?);
        }
        Ok(Rest {
            decycle,
            engine,
            ident,
            uses,
            items,
        })
    }
}

/// The `__discover!` callback: expand the rest of the module that [`start`] began, now that the
/// trampolines have said what the module's `#[decycle] use` items carry. Emits the module's body.
pub fn finish(input: TokenStream) -> TokenStream {
    let input: announce::helper::Input = match syn::parse2(input) {
        Ok(input) => input,
        Err(e) => abort!(e.span(), "{}", e),
    };
    let rest: Rest =
        match syn::parse::Parser::parse2(|input: ParseStream| input.parse(), input.rest.clone()) {
            Ok(rest) => rest,
            Err(e) => abort!(e.span(), "{}", e),
        };
    // An error below leaves the module's items as the user wrote them, so that code which names them
    // does not fail too.
    let items = &rest.items;
    set_dummy(quote! { #(#items)* });
    let renames: Vec<(Ident, Ident)> = rest
        .uses
        .iter()
        .flat_map(|u| crate::ranked::process_module::collect_trait_renames(&Item::Use(u.clone())))
        .collect();
    let carried = input
        .entries
        .iter()
        .map(|entry| crate::ranked::carried::entry_name(entry, &renames))
        .collect();
    let module = ItemMod {
        attrs: Vec::new(),
        vis: Visibility::Inherited,
        unsafety: None,
        mod_token: Default::default(),
        ident: rest.ident,
        content: Some((Default::default(), rest.items)),
        semi: None,
    };
    let discovered = Discovered {
        carried,
        uses: rest.uses,
    };
    match rest.engine {
        Engine::Ranked {
            recurse_level,
            support_infinite_cycle,
        } => crate::ranked::process_module::process_module_discovered(
            module,
            &rest.decycle,
            recurse_level,
            support_infinite_cycle,
            &discovered,
        ),
        Engine::Structural => {
            crate::structural::process_module_discovered(module, &rest.decycle, &discovered)
        }
    }
}
