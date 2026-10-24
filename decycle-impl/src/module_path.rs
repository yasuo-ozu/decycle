//! `#[decycle(module = path)]`: the absolute path of the annotated module.
//!
//! The engines read the module as written, and a cycle participant only by the name the module
//! gives it, bare or `self::`-qualified: a bound `Term: Eval` is an edge of the cycle, and
//! `crate::m::Term: Eval` (or `Term: crate::m::Eval`, or an `impl Eval for crate::m::Term`) is not —
//! a path the module writes names the item itself, the opt-out. An item that `#[announce]` re-reads
//! elsewhere (a struct or enum another attribute announces, say) must instead spell every path
//! absolutely, so the two requirements used to collide inside a `#[decycle]` module. Given the
//! module's own path, [`ModuleScope::qualify_item`] reconciles them, once the cycle is solved: the
//! items an engine re-emits as written (the structs, enums, unions, traits, type aliases, fns,
//! consts and statics, and every impl it does not take over) get each bare (or `self::`) name of a
//! type or trait declared directly in the module re-spelled `<module>::X`, so an `#[announce]` on
//! one of them, which expands after the module, sees absolute paths only (and an absolute path the
//! module writes stays as written).
//!
//! The impls an engine takes over are generated anew, partly inside helper modules where a bare
//! trait name deliberately resolves to something else, so they keep the bare spelling.
//!
//! The rewrite is syntactic and conservative. It touches only paths in type and trait-bound
//! positions (field types, the types of consts and statics, generics and their bounds, where
//! clauses, supertraits, signatures, impl headers, associated types), never an expression, a
//! pattern, a body, an attribute, a macro's tokens or a nested module. A bare name shadowed by a
//! generic parameter in scope is left alone, as is `Self` and any other path of more than one
//! segment. Being syntactic, it has two blind spots:
//!
//! - **`#[cfg]` is not evaluated**: a type declared under an inactive `#[cfg]` still counts as
//!   declared, so its name is qualified wherever the module uses it — even where it means the
//!   prelude's item of that name (a `#[cfg(any())] struct String;` turns every `String` into
//!   `<module>::String`, which then does not exist).
//! - **macro-generated items are invisible**: a type a `macro_rules!` invocation declares in the
//!   module is not seen as declared, so a bare use of it is left bare.
//!
//! [`ModuleScope::path_check`] checks the path against the module: no syntactic rule can tell a
//! wrong but well-formed path from the right one.

use proc_macro2::{Span, TokenStream};
use proc_macro_error::abort;
use std::collections::HashSet;
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::visit_mut::VisitMut;
use syn::*;
use template_quote::{quote, ToTokens};

/// The `module = path` of a `#[decycle]` module, with the names it re-spells.
pub(crate) struct ModuleScope {
    /// The module's absolute path, as written in the attribute.
    path: Path,
    /// Types and traits declared directly in the module: struct, enum, union, type alias, trait and
    /// trait alias. These are what [`ModuleScope::qualify_item`] makes absolute.
    declared: HashSet<String>,
    /// The nominal types among them (struct, enum, union): the ones a path can name with no doubt
    /// about which type it is, unlike an alias. See [`ModuleScope::own_type_by_path`].
    nominal: HashSet<String>,
    /// The nonce-named marker item [`ModuleScope::path_check`] declares in the module.
    marker: Ident,
}

/// A segment's name for comparison: `unraw`, and `$crate` (what a `macro_rules!` passes for
/// `$crate`) read as `crate`.
fn seg_name(ident: &Ident) -> String {
    let name = ident.unraw().to_string();
    if name == "$crate" {
        "crate".to_owned()
    } else {
        name
    }
}

impl ModuleScope {
    /// Validate `path` as the path of `module` and collect the module's names.
    ///
    /// Aborts (through `proc_macro_error`) when `path` is not absolute (`crate::..`, `$crate::..` or
    /// `::..`), carries generic arguments, or does not end in the module's own name.
    pub(crate) fn new(path: &Path, module: &ItemMod) -> Self {
        check_module_path(path, &module.ident);
        let mut declared = HashSet::new();
        let mut nominal = HashSet::new();
        for item in module.content.iter().flat_map(|(_, items)| items) {
            let (ident, is_nominal) = match item {
                Item::Struct(it) => (&it.ident, true),
                Item::Enum(it) => (&it.ident, true),
                Item::Union(it) => (&it.ident, true),
                Item::Type(it) => (&it.ident, false),
                Item::Trait(it) => (&it.ident, false),
                Item::TraitAlias(it) => (&it.ident, false),
                _ => continue,
            };
            if is_nominal {
                nominal.insert(seg_name(ident));
            }
            declared.insert(seg_name(ident));
        }
        let nonce = crate::identity_to_u64(&module.to_token_stream().to_string());
        let marker = crate::suffixed_ident("__DecycleModule_", nonce);
        ModuleScope {
            path: path.clone(),
            declared,
            nominal,
            marker,
        }
    }

    /// The name of the module's own struct, enum or union that `path` names **by a path**:
    /// `<module>::X`, or `super::<name>::X`, which is the same path once `super` is resolved
    /// against the module path. `None` for every other path: bare `X` and `self::X` are the
    /// module's name for the item, not a path to it; a path to another module, to a re-export or
    /// to an import, or one with generic arguments, does not provably name this module's `X`.
    ///
    /// The comparison is by segment name (`$crate` reads as `crate`, `r#X` as `X`), and only proves
    /// something together with [`ModuleScope::path_check`], which rejects a module path that is
    /// not this module's.
    pub(crate) fn own_type_by_path(&self, path: &Path) -> Option<String> {
        if path.segments.iter().any(|seg| !seg.arguments.is_none()) {
            return None;
        }
        let module: Vec<String> = self
            .path
            .segments
            .iter()
            .map(|s| seg_name(&s.ident))
            .collect();
        let written: Vec<String> = path.segments.iter().map(|s| seg_name(&s.ident)).collect();
        let resolved = match written.split_first() {
            // `super::m::X` inside `crate::..::m`: `super` is the module path without its last
            // segment. (A second `super` stays a `super` and matches nothing; the ranked engine
            // rejects that spelling anyway.)
            Some((first, rest)) if first == "super" && path.leading_colon.is_none() => {
                let parent = &module[..module.len() - 1];
                if parent.is_empty() {
                    return None;
                }
                parent.iter().chain(rest).cloned().collect()
            }
            _ if path.leading_colon.is_some() != self.path.leading_colon.is_some() => return None,
            _ => written,
        };
        let (name, prefix) = resolved.split_last()?;
        (prefix == module.as_slice() && self.nominal.contains(name)).then(|| name.clone())
    }

    /// Is `ident` (bare) the name of a struct, enum or union declared directly in the module?
    pub(crate) fn declares_nominal(&self, ident: &Ident) -> bool {
        self.nominal.contains(&seg_name(ident))
    }

    /// The check that `path` is this module's own: a nonce-named marker item in the module, and a
    /// use of it through `path`, spanned at the `module` argument.
    ///
    /// Nothing else can tell a wrong but well-formed path from the right one: `module = crate::m` on
    /// a module that is really `crate::outer::m` still names a module, and when that module
    /// declares a `Leaf` too, the qualified items quietly use it instead of this module's. Through
    /// the check, a path that is not this module's fails once, at the argument (``cannot find type
    /// `__DecycleModule_..` in module `crate::m` ``), whether or not it names a module at all. The
    /// marker lives in this module only, so no other path reaches it; `::core` keeps the check
    /// usable from a `#![no_std]` crate.
    pub(crate) fn path_check(&self) -> TokenStream {
        let marker = &self.marker;
        let at = self
            .path
            .segments
            .last()
            .map_or_else(Span::call_site, |seg| seg.ident.span());
        let marker_at = Ident::new(&marker.to_string(), Span::call_site().located_at(at));
        let path = &self.path;
        quote! {
            #[doc(hidden)]
            #[allow(dead_code, non_camel_case_types)]
            struct #marker;
            const _: () = {
                let _: ::core::option::Option<#path::#marker_at> = ::core::option::Option::None;
            };
        }
    }

    /// `item`, with every bare name of a type or trait declared in the module re-spelled
    /// `<module>::X` in its type and trait positions.
    pub(crate) fn qualify_item(&self, item: &Item) -> Item {
        let mut item = item.clone();
        self.qualify(&mut item);
        item
    }

    /// [`ModuleScope::qualify_item`] over every direct item of `module`.
    pub(crate) fn qualify_module(&self, module: &mut ItemMod) {
        if let Some((_, items)) = &mut module.content {
            for item in items {
                self.qualify(item);
            }
        }
    }

    fn qualify(&self, item: &mut Item) {
        match item {
            // A nested module has its own names, and a macro its own syntax; a `use`/`extern crate`
            // names no type position.
            Item::Mod(_) | Item::Macro(_) | Item::Use(_) | Item::ExternCrate(_) => {}
            _ => Qualifier {
                scope: self,
                shadowed: Vec::new(),
            }
            .visit_item_mut(item),
        }
    }

    /// `X` (the segment, with its arguments) as `<module>::X`.
    ///
    /// The path's FIRST token (the leading `::`, or else `crate`/`$crate`) is relocated to `seg`'s
    /// position, so the qualified path spans exactly the name the user wrote, and a diagnostic about
    /// the type points there; its hygiene is kept (`located_at`), which is what makes a `$crate`
    /// from a `macro_rules!` resolve. The other tokens of the module path keep their place in the
    /// `module` argument: a diagnostic about one of them is about the argument (a module that does
    /// not exist), and pointing every qualified name at the same token makes rustc report it once,
    /// next to the [`ModuleScope::path_check`] error, instead of once per name.
    fn qualified(&self, seg: PathSegment) -> Path {
        let span = seg.ident.span();
        let relocate = |s: Span| s.located_at(span);
        let leading_colon = self
            .path
            .leading_colon
            .map(|c| Token![::](relocate(c.spans[0])));
        let mut segments = Punctuated::new();
        for (i, pair) in self.path.segments.pairs().enumerate() {
            let mut module_seg = PathSegment::clone(pair.value());
            if i == 0 && leading_colon.is_none() {
                module_seg.ident.set_span(relocate(module_seg.ident.span()));
            }
            segments.push_value(module_seg);
            segments.push_punct(pair.punct().map_or(Token![::](span), |p| **p));
        }
        segments.push_value(seg);
        Path {
            leading_colon,
            segments,
        }
    }
}

/// Why a `module = path` cannot be the path of its module; see [`module_path_error`].
#[doc(hidden)]
pub struct ModulePathError {
    /// The offending part of the path, for the error's span.
    pub at: TokenStream,
    pub message: String,
    pub help: String,
}

/// Why `path` cannot be the `module = path` of the module `module_ident`, or `None` if it can.
///
/// The path must be absolute (its first segment `crate`, or `$crate` as a `macro_rules!` passes it,
/// or a leading `::`), carry no generic arguments, and end in the module's own name. Exposed (hidden)
/// so that the attribute can expand the module without the path before it reports the error, which
/// keeps the error alone (see `decycle-macro`).
#[doc(hidden)]
pub fn module_path_error(path: &Path, module_ident: &Ident) -> Option<ModulePathError> {
    let module_name = module_ident.unraw();
    let first = path.segments.first().map(|seg| seg_name(&seg.ident));
    if path.leading_colon.is_none() && first.as_deref() != Some("crate") {
        return Some(ModulePathError {
            at: path.to_token_stream(),
            message: "`module` must be the absolute path of this module".to_owned(),
            help: format!("start it at the crate root: `module = crate::..::{module_name}`"),
        });
    }
    if let Some(seg) = path.segments.iter().find(|seg| !seg.arguments.is_none()) {
        return Some(ModulePathError {
            at: seg.arguments.to_token_stream(),
            message: "`module` is the path of a module, which takes no generic arguments"
                .to_owned(),
            help: format!("write the module path alone: `module = crate::..::{module_name}`"),
        });
    }
    let last = path.segments.last()?;
    if seg_name(&last.ident) != seg_name(module_ident) {
        return Some(ModulePathError {
            at: last.ident.to_token_stream(),
            message: format!(
                "`module` must name the module it is on: it ends in `{}`, but the module is `{}`",
                last.ident.unraw(),
                module_name
            ),
            help: format!("write this module's own path, ending in `{module_name}`"),
        });
    }
    None
}

/// [`module_path_error`], as an abort.
fn check_module_path(path: &Path, module_ident: &Ident) {
    if let Some(err) = module_path_error(path, module_ident) {
        abort!(err.at, "{}", err.message; help = "{}", err.help);
    }
}

/// Is `seg` the no-op `self` prefix of a path (`self::X`)?
fn is_self_segment(seg: &PathSegment) -> bool {
    seg.ident == "self" && seg.arguments.is_none()
}

/// The rewrite of one item, `X` → `<module>::X`; see the module docs for what it touches.
struct Qualifier<'a> {
    scope: &'a ModuleScope,
    /// The generic parameters in scope (item, impl, method, GAT), innermost last. A parameter
    /// shadows a module item of the same name, which is left alone.
    shadowed: Vec<HashSet<String>>,
}

impl Qualifier<'_> {
    /// Enter the scope of `generics` (left with `self.shadowed.pop()`).
    fn enter(&mut self, generics: &Generics) {
        self.shadowed.push(
            crate::generics_fmt::type_const_idents(generics)
                .iter()
                .map(seg_name)
                .collect(),
        );
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.shadowed.iter().any(|scope| scope.contains(name))
    }

    /// Does the module's name `X` take `<module>::X` here: bare `X` (unless a generic parameter in
    /// scope shadows it), or `self::X`, which no generic parameter can shadow?
    fn qualifies(&self, name: &Ident, via_self: bool) -> bool {
        let name = seg_name(name);
        name != "Self"
            && self.scope.declared.contains(&name)
            && (via_self || !self.is_shadowed(&name))
    }

    /// Qualify a path in a type or trait position (no `qself`): `X` or `self::X`.
    fn qualify_path(&self, path: &mut Path) {
        if path.leading_colon.is_some() {
            return;
        }
        let via_self = match path.segments.len() {
            1 => false,
            2 if is_self_segment(&path.segments[0]) => true,
            _ => return,
        };
        if !self.qualifies(&path.segments.last().unwrap().ident, via_self) {
            return;
        }
        let seg = path.segments.pop().unwrap().into_value();
        *path = self.scope.qualified(seg);
    }

    /// Qualify the trait of a qualified path `<T as Trait>::Assoc` (or `<T as self::Trait>::Assoc`):
    /// the first `qself.position` segments of `path`.
    fn qualify_qself_trait(&self, qself: &mut QSelf, path: &mut Path) {
        if path.leading_colon.is_some() {
            return;
        }
        let via_self = match qself.position {
            1 => false,
            2 if is_self_segment(&path.segments[0]) => true,
            _ => return,
        };
        if !self.qualifies(&path.segments[qself.position - 1].ident, via_self) {
            return;
        }
        let mut pairs = std::mem::take(&mut path.segments).into_pairs();
        if via_self {
            pairs.next();
        }
        let trait_seg = pairs.next().unwrap().into_value();
        let mut qualified = self.scope.qualified(trait_seg);
        for pair in pairs {
            qualified
                .segments
                .push_punct(Token![::](pair.value().ident.span()));
            qualified.segments.push_value(pair.into_value());
        }
        *path = qualified;
        qself.position = self.scope.path.segments.len() + 1;
    }
}

impl VisitMut for Qualifier<'_> {
    // ----- type and trait positions -----

    fn visit_type_path_mut(&mut self, tp: &mut TypePath) {
        match &mut tp.qself {
            None => self.qualify_path(&mut tp.path),
            Some(qself) => self.qualify_qself_trait(qself, &mut tp.path),
        }
        syn::visit_mut::visit_type_path_mut(self, tp);
    }

    fn visit_trait_bound_mut(&mut self, tb: &mut TraitBound) {
        self.qualify_path(&mut tb.path);
        syn::visit_mut::visit_trait_bound_mut(self, tb);
    }

    fn visit_item_impl_mut(&mut self, im: &mut ItemImpl) {
        self.enter(&im.generics);
        if let Some((_, trait_path, _)) = &mut im.trait_ {
            self.qualify_path(trait_path);
        }
        syn::visit_mut::visit_item_impl_mut(self, im);
        self.shadowed.pop();
    }

    // ----- generic scopes -----

    fn visit_item_struct_mut(&mut self, it: &mut ItemStruct) {
        self.enter(&it.generics);
        syn::visit_mut::visit_item_struct_mut(self, it);
        self.shadowed.pop();
    }

    fn visit_item_enum_mut(&mut self, it: &mut ItemEnum) {
        self.enter(&it.generics);
        syn::visit_mut::visit_item_enum_mut(self, it);
        self.shadowed.pop();
    }

    fn visit_item_union_mut(&mut self, it: &mut ItemUnion) {
        self.enter(&it.generics);
        syn::visit_mut::visit_item_union_mut(self, it);
        self.shadowed.pop();
    }

    fn visit_item_type_mut(&mut self, it: &mut ItemType) {
        self.enter(&it.generics);
        syn::visit_mut::visit_item_type_mut(self, it);
        self.shadowed.pop();
    }

    fn visit_item_trait_mut(&mut self, it: &mut ItemTrait) {
        self.enter(&it.generics);
        syn::visit_mut::visit_item_trait_mut(self, it);
        self.shadowed.pop();
    }

    fn visit_item_trait_alias_mut(&mut self, it: &mut ItemTraitAlias) {
        self.enter(&it.generics);
        syn::visit_mut::visit_item_trait_alias_mut(self, it);
        self.shadowed.pop();
    }

    fn visit_trait_item_type_mut(&mut self, it: &mut TraitItemType) {
        self.enter(&it.generics);
        syn::visit_mut::visit_trait_item_type_mut(self, it);
        self.shadowed.pop();
    }

    fn visit_impl_item_type_mut(&mut self, it: &mut ImplItemType) {
        self.enter(&it.generics);
        syn::visit_mut::visit_impl_item_type_mut(self, it);
        self.shadowed.pop();
    }

    fn visit_foreign_item_type_mut(&mut self, it: &mut ForeignItemType) {
        self.enter(&it.generics);
        syn::visit_mut::visit_foreign_item_type_mut(self, it);
        self.shadowed.pop();
    }

    /// Every fn (free, trait method, impl method, foreign): its generics scope its inputs, output and
    /// where clause.
    fn visit_signature_mut(&mut self, sig: &mut Signature) {
        self.enter(&sig.generics);
        syn::visit_mut::visit_signature_mut(self, sig);
        self.shadowed.pop();
    }

    // ----- left as written -----

    fn visit_block_mut(&mut self, _: &mut Block) {}
    fn visit_expr_mut(&mut self, _: &mut Expr) {}
    fn visit_pat_mut(&mut self, _: &mut Pat) {}
    fn visit_macro_mut(&mut self, _: &mut Macro) {}
    fn visit_attribute_mut(&mut self, _: &mut Attribute) {}
    fn visit_item_mod_mut(&mut self, _: &mut ItemMod) {}
    fn visit_visibility_mut(&mut self, _: &mut Visibility) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(module: &ItemMod) -> Vec<String> {
        module
            .content
            .as_ref()
            .unwrap()
            .1
            .iter()
            .map(|item| item.to_token_stream().to_string())
            .collect()
    }

    fn qualified(module: ItemMod) -> Vec<String> {
        let mut module = module;
        ModuleScope::new(&parse_quote!(crate::m), &module).qualify_module(&mut module);
        items(&module)
    }

    #[test]
    fn qualifies_declared_names_in_type_and_trait_positions() {
        let module: ItemMod = parse_quote! {
            mod m {
                use super::Imported;
                pub struct Expr(pub Box<Term>, pub Option<Imported>);
                pub enum Term { Paren(Box<Expr>), Lit(i32) }
                pub type Pair<T> = (Expr, T, <Term as Kind>::Out);
                pub trait Kind: Base { type Out: Base; fn make(&self, e: &Expr) -> Term; }
                pub trait Base {}
                impl<T: Base> Kind for Wrap<T> where Term: Kind, Self: Base {
                    type Out = Expr;
                    fn make(&self, e: &Expr) -> Term { let t: Term = Term::Lit(1); t }
                }
                pub struct Wrap<T>(T);
                fn f(e: Expr, d: &dyn Base, i: impl Kind<Out = Term>) -> self::Term { todo!() }
                mod inner { pub struct Expr; pub fn g() -> Expr { Expr } }
            }
        };
        let expected: ItemMod = parse_quote! {
            mod m {
                use super::Imported;
                pub struct Expr(pub Box<crate::m::Term>, pub Option<Imported>);
                pub enum Term { Paren(Box<crate::m::Expr>), Lit(i32) }
                pub type Pair<T> = (crate::m::Expr, T, <crate::m::Term as crate::m::Kind>::Out);
                pub trait Kind: crate::m::Base { type Out: crate::m::Base; fn make(&self, e: &crate::m::Expr) -> crate::m::Term; }
                pub trait Base {}
                impl<T: crate::m::Base> crate::m::Kind for crate::m::Wrap<T> where crate::m::Term: crate::m::Kind, Self: crate::m::Base {
                    type Out = crate::m::Expr;
                    fn make(&self, e: &crate::m::Expr) -> crate::m::Term { let t: Term = Term::Lit(1); t }
                }
                pub struct Wrap<T>(T);
                fn f(e: crate::m::Expr, d: &dyn crate::m::Base, i: impl crate::m::Kind<Out = crate::m::Term>) -> crate::m::Term { todo!() }
                mod inner { pub struct Expr; pub fn g() -> Expr { Expr } }
            }
        };
        assert_eq!(qualified(module), items(&expected));
    }

    #[test]
    fn a_generic_parameter_shadows_a_declared_name() {
        let module: ItemMod = parse_quote! {
            mod m {
                pub struct Term;
                pub struct Holder<Term>(pub Term);
                impl<Term> Holder<Term> { fn get<U>(&self, u: U, t: crate::m::Term) -> Term { todo!() } }
                pub trait Pick { fn pick<Term>(&self, t: Term) -> Term; type Gat<Term>: Into<Term>; }
                pub fn free(t: Term) -> Term { t }
            }
        };
        let expected: ItemMod = parse_quote! {
            mod m {
                pub struct Term;
                pub struct Holder<Term>(pub Term);
                impl<Term> crate::m::Holder<Term> { fn get<U>(&self, u: U, t: crate::m::Term) -> Term { todo!() } }
                pub trait Pick { fn pick<Term>(&self, t: Term) -> Term; type Gat<Term>: Into<Term>; }
                pub fn free(t: crate::m::Term) -> crate::m::Term { t }
            }
        };
        assert_eq!(qualified(module), items(&expected));
    }

    #[test]
    fn qualifies_self_qualified_names() {
        let module: ItemMod = parse_quote! {
            mod m {
                pub struct Term;
                pub trait Kind { type Out; }
                pub struct Holder<Term>(pub Term, pub self::Term, pub Box<self::Term>);
                pub type Out = <self::Term as self::Kind>::Out;
                pub fn f<T: self::Kind>(t: self::Missing, u: self::Term::Assoc) {}
            }
        };
        let expected: ItemMod = parse_quote! {
            mod m {
                pub struct Term;
                pub trait Kind { type Out; }
                pub struct Holder<Term>(pub Term, pub crate::m::Term, pub Box<crate::m::Term>);
                pub type Out = <crate::m::Term as crate::m::Kind>::Out;
                pub fn f<T: crate::m::Kind>(t: self::Missing, u: self::Term::Assoc) {}
            }
        };
        assert_eq!(qualified(module), items(&expected));
    }

    #[test]
    fn a_path_to_an_own_type_is_the_module_path_and_a_declared_nominal_type() {
        let module: ItemMod = parse_quote! {
            mod m {
                pub struct C;
                pub enum E {}
                pub type Alias = C;
                pub trait Tr {}
                use crate::other::Imported;
            }
        };
        let own = |module_path: Path, path: Path| {
            ModuleScope::new(&module_path, &module).own_type_by_path(&path)
        };
        let at_m = |path: Path| own(parse_quote!(crate::m), path);
        assert_eq!(at_m(parse_quote!(crate::m::C)).as_deref(), Some("C"));
        assert_eq!(at_m(parse_quote!(crate::m::r#E)).as_deref(), Some("E"));
        assert_eq!(at_m(parse_quote!(super::m::C)).as_deref(), Some("C"));
        // The module's own spellings, other modules, aliases, traits, imports, arguments.
        let others: [Path; 12] = [
            parse_quote!(C),
            parse_quote!(self::C),
            parse_quote!(crate::C),
            parse_quote!(crate::other::C),
            parse_quote!(crate::m::inner::C),
            parse_quote!(::krate::m::C),
            parse_quote!(super::C),
            parse_quote!(super::super::m::C),
            parse_quote!(crate::m::Alias),
            parse_quote!(crate::m::Tr),
            parse_quote!(crate::m::Imported),
            parse_quote!(crate::m::C<u8>),
        ];
        for path in others {
            assert_eq!(at_m(path.clone()), None, "{}", path.to_token_stream());
        }
        // `super` is resolved against the module path, and a leading `::` must match it.
        let nested = parse_quote!(crate::outer::m);
        assert_eq!(own(nested, parse_quote!(super::m::C)).as_deref(), Some("C"));
        let rooted = || parse_quote!(::krate::m);
        assert_eq!(
            own(rooted(), parse_quote!(::krate::m::C)).as_deref(),
            Some("C")
        );
        assert_eq!(
            own(rooted(), parse_quote!(super::m::C)).as_deref(),
            Some("C")
        );
        assert_eq!(own(rooted(), parse_quote!(krate::m::C)), None);
    }

    #[test]
    fn module_path_must_be_absolute_plain_and_name_the_module() {
        let m: Ident = parse_quote!(m);
        let error = |path: Path| module_path_error(&path, &m).map(|e| e.message);
        assert!(error(parse_quote!(crate::m)).is_none());
        assert!(error(parse_quote!(crate::a::r#m)).is_none());
        assert!(error(parse_quote!(::krate::m)).is_none());
        assert!(error(parse_quote!(m)).unwrap().contains("absolute"));
        assert!(error(parse_quote!(self::m)).unwrap().contains("absolute"));
        assert!(error(parse_quote!(super::m)).unwrap().contains("absolute"));
        assert!(error(parse_quote!(crate::m<u8>))
            .unwrap()
            .contains("generic arguments"));
        assert!(error(parse_quote!(crate::n))
            .unwrap()
            .contains("ends in `n`"));
        assert!(error(parse_quote!(crate))
            .unwrap()
            .contains("ends in `crate`"));
    }
}
