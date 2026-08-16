//! Defuse the two name-resolution hazards created by *this crate's own* output.
//!
//! `finalize` re-emits every adopted impl inside helper modules nested one or two levels below the
//! module the user wrote (`shadowing_module`, `shadowing_module::ranked_traits`), and those modules
//! glob-import both enclosing scopes (`use super::*;` plus `use super::super::*;`). Two things break
//! as a result, and neither is the caller's fault:
//!
//! 1. **Ambiguity (`E0659`).** A bare name reachable through *both* globs — which is exactly the
//!    situation for every type defined in the processed module, since it is visible one level up as
//!    well — becomes ambiguous at the use site.
//! 2. **Relative paths change meaning.** `super::Marker` written in a where-clause means "one level
//!    above the module I am written in". After nesting it points one or two levels too shallow, so it
//!    silently resolves elsewhere or not at all.
//!
//! Both are fixed the same way: resolve the name *once*, at the level where the caller wrote it, bind
//! the result to an unambiguous alias, and refer to the alias from inside the nested impls. The
//! cycle-head aliases are emitted as ordinary `use` items in the processed module, where they resolve
//! exactly as the original spelling did; the nesting then cannot change what they mean. The lifted
//! relative paths instead live in a private `__DecycleRelMod_*` module and are referred to by a
//! two-segment path — a lifted path can name a TRAIT (`Foreign: super::x::Tr`, the documented
//! opt-out premise against the original, un-ranked trait), and a flat `use` of a trait would drop it
//! into the method-resolution scope of every re-emitted body, turning receiver calls ambiguous
//! (E0034) wherever a type implements both the original trait and its ranked twin.
//!
//! Alias names are **deterministic** (they carry the module's own ident, not a random nonce), so
//! diagnostics that mention one are stable enough to match in a golden test.

use proc_macro2::{Group, Ident, Spacing, TokenStream, TokenTree};
use proc_macro_error::abort;
use syn::spanned::Spanned;
use std::collections::HashSet;
use syn::visit_mut::VisitMut;
use syn::{
    ExprPath, GenericParam, Generics, Item, ItemImpl, Macro, Path, PathArguments, PathSegment,
    QSelf, TraitBound, TypePath,
};
use template_quote::quote;

/// Rewrite `adopted` so no bare cycle-type name and no relative path survives into the nested
/// modules, returning the `use` items that define the aliases introduced.
///
/// `local_cycle_heads` are the cycle types **defined in this module** — only those can be ambiguous
/// under the double glob, and only those can be named by a `use self::…` alias. A foreign type with a
/// cyclic impl here (`impl Tr for &str`) is reachable by exactly one path and needs no alias; trying
/// to bind one would be `E0432: unresolved import self::str`.
pub(crate) fn defuse_nesting(
    adopted: &mut [ItemImpl],
    local_cycle_heads: &HashSet<Ident>,
    cyclic_traits: &HashSet<Ident>,
    module_ident: &Ident,
) -> Vec<Item> {
    let mut items = Vec::new();

    // (1) Bind each locally-defined cycle type to a name only *this* module introduces.
    let mut heads: Vec<&Ident> = local_cycle_heads.iter().collect();
    heads.sort_by_key(|i| i.to_string());
    let aliases: Vec<(String, String)> = heads
        .iter()
        .map(|head| {
            // `unraw`, not the `Display` spelling: `Ident::new` panics on the `#` of a raw
            // identifier, so `struct r#loop` used to blow the macro up with an unspanned
            // `"__DecycleNat_r#loop_m" is not a valid identifier`.
            let name = format!(
                "__DecycleNat_{}_{}",
                crate::helper::unraw(head),
                crate::helper::unraw(module_ident)
            );
            let alias = Ident::new(&name, head.span());
            items.push(syn::parse_quote! {
                #[allow(non_camel_case_types, unused_imports)]
                use self::#head as #alias;
            });
            (head.to_string(), name)
        })
        .collect();
    for im in adopted.iter_mut() {
        AliasHeads {
            aliases: &aliases,
            shadowed: Vec::new(),
        }
        .visit_item_impl_mut(im);
    }

    // (2) Lift every `super::…` / `self::…` path to an alias resolved at this level.
    //
    // The aliases live in a dedicated private module, NOT as flat `use` items next to the user's
    // code: a `use super::x::Tr as Alias;` at module level puts the ORIGINAL trait `Tr` into the
    // method-resolution scope of every re-emitted body (the nested modules glob this module), and a
    // receiver implementing both the ranked twin and the original then failed with E0034
    // "multiple applicable items in scope". Behind `#rel_mod::`, an alias is nameable by path —
    // which is all bounds and qualified calls need — while its trait never enters any scope that
    // resolves a method call.
    let mut lifted: Vec<(Ident, Path)> = Vec::new();
    let rel_mod = Ident::new(
        &format!("__DecycleRelMod_{}", crate::helper::unraw(module_ident)),
        module_ident.span(),
    );
    for im in adopted.iter_mut() {
        LiftRelative {
            lifted: &mut lifted,
            cyclic_traits,
            module_ident,
            rel_mod: &rel_mod,
        }
        .visit_item_impl_mut(im);
    }
    if !lifted.is_empty() {
        let uses: Vec<Item> = lifted
            .iter()
            .map(|(alias, path)| {
                let deeper = one_module_deeper(path);
                syn::parse_quote! {
                    #[allow(non_camel_case_types, unused_imports)]
                    pub(super) use #deeper as #alias;
                }
            })
            .collect();
        items.push(syn::parse_quote! {
            #[allow(non_snake_case)]
            mod #rel_mod {
                #(#uses)*
            }
        });
    }

    items
}

/// `path`, re-spelled to resolve from one module further down (inside the alias module): the
/// leading `self` becomes `super`, a leading `super` gains one more.
fn one_module_deeper(path: &Path) -> Path {
    let mut path = path.clone();
    let root = &mut path.segments[0].ident;
    let root_span = root.span();
    if *root == "self" {
        *root = Ident::new("super", root_span);
    } else {
        debug_assert_eq!(*root, "super");
        path.segments.insert(
            0,
            PathSegment {
                ident: Ident::new("super", root_span),
                arguments: PathArguments::None,
            },
        );
    }
    path
}

/// Re-spell the leading segment of any non-`::`-rooted path that names a local cycle type.
///
/// The replacement ident is stamped with the span of the ident it replaces, not `call_site` — an
/// alias is an internal detail, and a diagnostic mentioning one should still point at the line the
/// caller wrote rather than at the `#[decycle]` attribute.
///
/// **Scope-aware**: a generic parameter of the impl (or of a method / nested fn) that happens to
/// share a cycle-head name SHADOWS it, so every path headed by that parameter is left alone —
/// re-spelling it used to rewrite the PARAMETER into the STRUCT (E0207 + phantom `Copy` bounds on
/// the struct). `shadowed` is a stack of the generic-param name sets currently in scope.
struct AliasHeads<'a> {
    aliases: &'a [(String, String)],
    shadowed: Vec<HashSet<String>>,
}

impl AliasHeads<'_> {
    fn generic_idents(generics: &Generics) -> HashSet<String> {
        generics
            .params
            .iter()
            .filter_map(|p| match p {
                GenericParam::Type(t) => Some(t.ident.to_string()),
                GenericParam::Const(c) => Some(c.ident.to_string()),
                GenericParam::Lifetime(_) => None,
            })
            .collect()
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.shadowed.iter().any(|scope| scope.contains(name))
    }

    fn alias_for(&self, name: &str) -> Option<&str> {
        if self.is_shadowed(name) {
            return None;
        }
        self.aliases
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, a)| a.as_str())
    }
}

impl VisitMut for AliasHeads<'_> {
    fn visit_item_impl_mut(&mut self, im: &mut ItemImpl) {
        self.shadowed.push(Self::generic_idents(&im.generics));
        syn::visit_mut::visit_item_impl_mut(self, im);
        self.shadowed.pop();
    }

    fn visit_impl_item_fn_mut(&mut self, f: &mut syn::ImplItemFn) {
        self.shadowed.push(Self::generic_idents(&f.sig.generics));
        syn::visit_mut::visit_impl_item_fn_mut(self, f);
        self.shadowed.pop();
    }

    fn visit_item_fn_mut(&mut self, f: &mut syn::ItemFn) {
        // A nested `fn g<Stmt>()` inside a method body shadows too.
        self.shadowed.push(Self::generic_idents(&f.sig.generics));
        syn::visit_mut::visit_item_fn_mut(self, f);
        self.shadowed.pop();
    }

    fn visit_path_mut(&mut self, path: &mut Path) {
        if path.leading_colon.is_none() {
            if let Some(first) = path.segments.first_mut() {
                let name = first.ident.to_string();
                if let Some(alias) = self.alias_for(&name) {
                    first.ident = Ident::new(alias, first.ident.span());
                }
            }
        }
        syn::visit_mut::visit_path_mut(self, path);
    }

    /// `syn` never descends into a macro's token stream, so a cycle-head name inside
    /// `matches!(self, Stmt::Leaf)` used to survive un-aliased and hit the double-glob
    /// ambiguity (E0659) whenever an outer scope also defines the name. Rewriting arbitrary
    /// macro input is NOT safe in general — `stringify!` and friends treat idents as data —
    /// so only macros on a known allowlist (std macros whose input is ordinary
    /// expression/pattern code) have their tokens rewritten; everything else is left exactly
    /// as written, which is at worst the pre-existing loud ambiguity error, never a silent
    /// change of meaning.
    fn visit_macro_mut(&mut self, mac: &mut Macro) {
        if macro_input_is_expression_code(&mac.path) {
            mac.tokens = self.rewrite_macro_tokens(std::mem::take(&mut mac.tokens));
        }
        syn::visit_mut::visit_macro_mut(self, mac);
    }
}

/// Std macros whose input tokens are ordinary expression / pattern code, so an ident in path
/// position inside them refers to the item exactly like it would outside a macro. Notably
/// ABSENT: `stringify!`, `concat_idents!`, `cfg!`, `env!`, `include!` — anything that reads
/// idents as data. Format-family macros are safe because their format string is a `Literal`
/// token, untouched by the ident rewrite.
const EXPRESSION_CODE_MACROS: &[&str] = &[
    "matches",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "vec",
    "format",
    "format_args",
    "write",
    "writeln",
    "print",
    "println",
    "eprint",
    "eprintln",
    "dbg",
];

/// Is this macro path a plausible spelling of an allowlisted std macro — the bare name, or the
/// name rooted at `std`/`core`/`alloc`? (A same-named user macro imported into scope is treated
/// like the std one; shadowing a prelude macro name is rare and linted against, and the
/// alternative — never rewriting — regresses the overwhelmingly common `matches!` case.)
fn macro_input_is_expression_code(path: &Path) -> bool {
    let Some(last) = path.segments.last() else {
        return false;
    };
    if !EXPRESSION_CODE_MACROS.contains(&last.ident.to_string().as_str()) {
        return false;
    }
    match path.segments.len() {
        1 => path.leading_colon.is_none(),
        2 => {
            let root = path.segments[0].ident.to_string();
            root == "std" || root == "core" || root == "alloc"
        }
        _ => false,
    }
}

impl AliasHeads<'_> {
    /// Rewrite cycle-head idents inside an (allowlisted) macro's token stream, conservatively:
    ///
    /// - an ident preceded by `:` (path continuation `Expr::Stmt`), `.` (field access), `$`
    ///   (macro-rules var), or `'` (lifetime) is never touched;
    /// - an ident followed by a *single* `:` (field-init / binding position, `Stmt: 1`) is never
    ///   touched — but `Stmt::…` (joint first colon) is a path head and IS rewritten;
    /// - an ident followed by `!` is a nested macro name: the name itself is never rewritten, and
    ///   its argument group is recursed into only when that macro is allowlisted too (so a nested
    ///   `stringify!(Stmt)` inside an `assert_eq!` stays verbatim);
    /// - shadowing generic params suppress the rewrite exactly as outside macros.
    fn rewrite_macro_tokens(&self, tokens: TokenStream) -> TokenStream {
        let toks: Vec<TokenTree> = tokens.into_iter().collect();
        let mut out: Vec<TokenTree> = Vec::with_capacity(toks.len());
        let mut i = 0;
        while i < toks.len() {
            match &toks[i] {
                TokenTree::Ident(id) => {
                    let followed_by = toks.get(i + 1);
                    if matches!(followed_by, Some(TokenTree::Punct(p)) if p.as_char() == '!') {
                        // A nested macro call: `name ! <group>` (or `name !` alone — `!=` never
                        // follows a bare ident-in-expression as one Punct pair here because
                        // `assert!(a != b)` lexes `!=` as its own joint punct, not after an
                        // ident... it does follow: `a != b` — so require a Group right after
                        // to treat it as a macro call).
                        if let Some(TokenTree::Group(g)) = toks.get(i + 2) {
                            out.push(toks[i].clone());
                            out.push(toks[i + 1].clone());
                            let nested_allowlisted = EXPRESSION_CODE_MACROS
                                .contains(&id.to_string().as_str());
                            if nested_allowlisted {
                                let mut ng =
                                    Group::new(g.delimiter(), self.rewrite_macro_tokens(g.stream()));
                                ng.set_span(g.span());
                                out.push(TokenTree::Group(ng));
                            } else {
                                out.push(toks[i + 2].clone());
                            }
                            i += 3;
                            continue;
                        }
                    }
                    let prev_blocks = matches!(
                        out.last(),
                        Some(TokenTree::Punct(p))
                            if matches!(p.as_char(), ':' | '.' | '$' | '\'')
                    );
                    let next_is_single_colon = matches!(
                        followed_by,
                        Some(TokenTree::Punct(p))
                            if p.as_char() == ':' && p.spacing() == Spacing::Alone
                    );
                    let name = id.to_string();
                    match self.alias_for(&name) {
                        Some(alias) if !prev_blocks && !next_is_single_colon => {
                            out.push(TokenTree::Ident(Ident::new(alias, id.span())));
                        }
                        _ => out.push(toks[i].clone()),
                    }
                }
                TokenTree::Group(g) => {
                    let mut ng = Group::new(g.delimiter(), self.rewrite_macro_tokens(g.stream()));
                    ng.set_span(g.span());
                    out.push(TokenTree::Group(ng));
                }
                other => out.push(other.clone()),
            }
            i += 1;
        }
        out.into_iter().collect()
    }
}

/// Replace a `super::…` / `self::…` rooted path with a `#rel_mod::#alias` path, recording the
/// original so the caller can bind it. Generic arguments stay on the alias
/// (`super::Wrap<T>` ⇒ `__DecycleRelMod_m::Alias<T>`), since only the *path* needs resolving, not
/// the instantiation.
///
/// Deliberately narrow — three kinds of path are left exactly as written:
/// - **cycle edges**: a BARE reference to a routed trait, or its no-op `self::`-qualified form.
///   That spelling is the ranked engine's own signal (already normalised by `strip_leading_self`);
///   hiding it behind an alias would make a cycle edge look like an ordinary premise. A
///   longer-qualified reference to the same trait (`Foreign: super::x::Tr`) is the documented
///   opt-out that binds against the ORIGINAL, un-ranked trait — an ordinary premise, so it IS
///   lifted. (Refusing on the last segment alone left the opt-out spelling behind, where nesting
///   turned it into E0433 — the same last-segment confusion `path_names_local_ident` exists to
///   prevent.)
/// - **whole qualified paths** (`<B as self::Tr>::Assoc`). Their `qself.position` indexes into
///   `segments`, so collapsing the path to a single segment corrupts the syntax tree. The trait
///   sub-path before `as` is an importable item though, and is lifted on its own — a re-emitted
///   body's `<String as super::x::Tr>::f(..)` needs exactly that.
///
/// A path in **value position** (an expression, a struct literal, a pattern) is lifted too, but only
/// by its two-segment PREFIX — see [`LiftRelative::lift_value_path`]. Leaving those "to resolve
/// normally" was the one silent-wrong case in this file: they do resolve, to a *different* item.
struct LiftRelative<'a> {
    lifted: &'a mut Vec<(Ident, Path)>,
    cyclic_traits: &'a HashSet<Ident>,
    module_ident: &'a Ident,
    rel_mod: &'a Ident,
}

impl LiftRelative<'_> {
    /// Returns whether `path` was rewritten (to `#rel_mod::#alias`).
    fn lift(&mut self, path: &mut Path) -> bool {
        if path.leading_colon.is_some() || path.segments.len() < 2 {
            return false;
        }
        let root = path.segments[0].ident.to_string();
        if root != "super" && root != "self" {
            return false;
        }
        let mut bare = path.clone();
        let args = std::mem::replace(
            &mut bare.segments.last_mut().unwrap().arguments,
            PathArguments::None,
        );
        let key = quote!(#bare).to_string();
        let span = path.segments[0].ident.span();
        let alias = match self
            .lifted
            .iter()
            .find(|(_, p)| quote!(#p).to_string() == key)
        {
            // Re-stamped with THIS site's span — see `AliasHeads`.
            Some((a, _)) => Ident::new(&a.to_string(), span),
            None => {
                let a = Ident::new(
                    &format!(
                        "__DecycleRelPath_{}_{}",
                        self.lifted.len(),
                        crate::helper::unraw(self.module_ident)
                    ),
                    span,
                );
                self.lifted.push((a.clone(), bare));
                a
            }
        };
        *path = Path {
            leading_colon: None,
            segments: [
                PathSegment {
                    ident: Ident::new(&self.rel_mod.to_string(), span),
                    arguments: PathArguments::None,
                },
                PathSegment {
                    ident: alias,
                    arguments: args,
                },
            ]
            .into_iter()
            .collect(),
        };
        true
    }

    /// Is `path` the spelling that marks a cycle edge — the bare name of a routed trait, or its
    /// no-op `self::`-qualified form? Only those participate in ranking (the same bare-or-`self::`
    /// rule `peel::is_bare_cyclic_bound` applies), so only those must survive un-aliased for the
    /// downstream rewrites to recognise them. A longer-qualified reference (`super::x::Tr`)
    /// deliberately names the original, un-ranked trait and is lifted like any other premise.
    fn spells_cycle_edge(&self, path: &Path) -> bool {
        crate::helper::path_names_local_ident(path, self.cyclic_traits)
    }

    /// Lift a path written in **value position** — an expression, a struct literal, or a pattern.
    ///
    /// Only the two-segment PREFIX (`super::X` / `self::X`) is bound to an alias; everything after
    /// it rides along unchanged. That is what makes this work where lifting the whole path cannot:
    /// a value path may end in something `use` is unable to import (`super::x::Tr::f` ends in an
    /// associated fn, `super::Type::CONST` in an associated const), but its second segment always
    /// names an item that `use` CAN import — a module, a type, an enum, a function, a constant —
    /// because that is the only thing a `super::`-rooted path's first named segment can be. Once it
    /// is reached through `#rel_mod::#alias`, the rest of the path resolves off it exactly as
    /// before.
    ///
    /// This used to be skipped entirely, on the grounds that a body path resolves normally. It does
    /// resolve — to the WRONG item. `finalize` re-emits the impl one or two modules deeper, where
    /// `super::` names the processed module (which glob-imports its own parent), so
    /// `super::helper()` silently called the module's own `helper` instead of the crate root's.
    /// Silent, because the module usually does define the shadowing name; when it does not, the
    /// error names an innocent `super::`.
    fn lift_value_path(&mut self, path: &mut Path) {
        if path.leading_colon.is_some() || path.segments.len() < 2 {
            return;
        }
        let root = path.segments[0].ident.to_string();
        if root != "super" && root != "self" {
            return;
        }
        let mut prefix = Path {
            leading_colon: None,
            segments: path.segments.iter().take(2).cloned().collect(),
        };
        // `self::Tr::f(..)` — the bare-or-`self::` spelling of a routed trait is the engine's own
        // cycle-edge signal, which downstream rewrites must still recognise. Left as written, like
        // every other occurrence of that spelling.
        if self.spells_cycle_edge(&prefix) {
            return;
        }
        if !self.lift(&mut prefix) {
            return;
        }
        let tail = path.segments.iter().skip(2).cloned();
        path.segments = prefix.segments.into_iter().chain(tail).collect();
    }

    /// Lift the TRAIT sub-path of a qualified path: `<String as super::x::Tr>::f` becomes
    /// `<String as __DecycleRelMod_m::__DecycleRelPath_N_m>::f`. The whole path cannot be
    /// collapsed — `qself.position` indexes into `segments` — but the segments before `as` name an
    /// importable trait, so they alone are replaced and the position adjusted.
    fn lift_qself(&mut self, qself: &mut QSelf, path: &mut Path) {
        if qself.position < 2 || path.leading_colon.is_some() {
            return;
        }
        let mut trait_path = Path {
            leading_colon: None,
            segments: path.segments.iter().take(qself.position).cloned().collect(),
        };
        if self.spells_cycle_edge(&trait_path) {
            return;
        }
        if !self.lift(&mut trait_path) {
            // Not depth-fragile (`crate::x::Tr`, `x::Tr`) — left as written.
            return;
        }
        let tail = path.segments.iter().skip(qself.position).cloned();
        let position = trait_path.segments.len();
        path.segments = trait_path.segments.into_iter().chain(tail).collect();
        qself.position = position;
    }
}

impl VisitMut for LiftRelative<'_> {
    fn visit_type_path_mut(&mut self, tp: &mut TypePath) {
        // Depth-first, so an inner relative argument is lifted before its enclosing path.
        syn::visit_mut::visit_type_path_mut(self, tp);
        if let Some(qself) = &mut tp.qself {
            self.lift_qself(qself, &mut tp.path);
            return;
        }
        if self.spells_cycle_edge(&tp.path) {
            return;
        }
        self.lift(&mut tp.path);
    }

    fn visit_trait_bound_mut(&mut self, tb: &mut TraitBound) {
        syn::visit_mut::visit_trait_bound_mut(self, tb);
        if self.spells_cycle_edge(&tb.path) {
            return;
        }
        self.lift(&mut tb.path);
    }

    // A qualified call (`<String as super::x::Tr>::f(..)`) has its trait sub-path lifted whole;
    // any other expression path is lifted by its prefix.
    fn visit_expr_path_mut(&mut self, ep: &mut ExprPath) {
        syn::visit_mut::visit_expr_path_mut(self, ep);
        match &mut ep.qself {
            Some(qself) => self.lift_qself(qself, &mut ep.path),
            None => self.lift_value_path(&mut ep.path),
        }
    }

    // The remaining value-position paths syn models with a `Path` field of their own. (A bare
    // *path pattern* is not among them: syn 2 models `Pat::Path` as an `ExprPath`, so
    // `super::Tag::Real` in a `match` arm goes through the override above.) All of them resolve
    // exactly as an expression path does — a struct literal naming `super::Outer` used to build the
    // processed module's own `Outer` after re-emission.
    fn visit_expr_struct_mut(&mut self, es: &mut syn::ExprStruct) {
        syn::visit_mut::visit_expr_struct_mut(self, es);
        if es.qself.is_none() {
            self.lift_value_path(&mut es.path);
        }
    }

    fn visit_pat_struct_mut(&mut self, ps: &mut syn::PatStruct) {
        syn::visit_mut::visit_pat_struct_mut(self, ps);
        if ps.qself.is_none() {
            self.lift_value_path(&mut ps.path);
        }
    }

    fn visit_pat_tuple_struct_mut(&mut self, pt: &mut syn::PatTupleStruct) {
        syn::visit_mut::visit_pat_tuple_struct_mut(self, pt);
        if pt.qself.is_none() {
            self.lift_value_path(&mut pt.path);
        }
    }

    /// A `super::`/`self::`-rooted path inside a macro invocation cannot be lifted: `syn` hands the
    /// invocation over as an opaque token stream, and rewriting one is only sound for the handful of
    /// std macros `AliasHeads` allowlists — an ident there may be data (`stringify!`), and a path
    /// prefix rewritten inside `concat_idents!`/`env!` would change meaning outright.
    ///
    /// Left alone it is the silent-wrong case again, so it is rejected instead: loud, at the macro
    /// the caller wrote, with the fix in the message.
    fn visit_macro_mut(&mut self, mac: &mut Macro) {
        if let Some(span) = relative_path_root_in_tokens(&mac.tokens) {
            abort!(
                span,
                "decycle: a `super::`/`self::`-rooted path inside a macro invocation is not supported in a #[decycle] impl body";
                help = "decycle re-emits this impl inside a generated helper module, where the path would silently name a different item — and a macro's tokens cannot be rewritten safely. Use a `crate::`-rooted path instead, or bind the item to a name outside the macro first."
            )
        }
        syn::visit_mut::visit_macro_mut(self, mac);
    }

    /// Same reasoning for a `use super::…;` / `use self::…;` written INSIDE a method body: it is not
    /// a `Path` (syn models it as a `UseTree`), and a `use` item cannot be re-rooted at the alias
    /// module — a 2018-edition `use` path has to start with `crate`/`self`/`super`/`::`/a crate
    /// name, so there is no spelling of `#rel_mod::#alias` that works there.
    fn visit_item_use_mut(&mut self, iu: &mut syn::ItemUse) {
        if let Some(span) = relative_use_root(&iu.tree) {
            abort!(
                span,
                "decycle: a `use super::…;` / `use self::…;` inside a #[decycle] impl body is not supported";
                help = "decycle re-emits this impl inside a generated helper module, where the import would silently resolve against a different module. Use a `crate::`-rooted import instead, or move it out to the module level."
            )
        }
        syn::visit_mut::visit_item_use_mut(self, iu);
    }
}

/// The span of a `super`/`self` token that starts a path (i.e. is followed by `::`) anywhere in
/// `tokens`, including inside nested groups.
///
/// `Self` is deliberately not matched: it is impl-relative and travels with the impl, so it means
/// the same thing at any depth.
fn relative_path_root_in_tokens(tokens: &TokenStream) -> Option<proc_macro2::Span> {
    let toks: Vec<TokenTree> = tokens.clone().into_iter().collect();
    for (i, tt) in toks.iter().enumerate() {
        match tt {
            TokenTree::Ident(id) => {
                let name = id.to_string();
                if (name == "super" || name == "self")
                    && matches!(toks.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == ':')
                {
                    return Some(id.span());
                }
            }
            TokenTree::Group(g) => {
                if let Some(span) = relative_path_root_in_tokens(&g.stream()) {
                    return Some(span);
                }
            }
            _ => {}
        }
    }
    None
}

/// The span of a `use` tree's leading `super`/`self` segment, if it has one.
fn relative_use_root(tree: &syn::UseTree) -> Option<proc_macro2::Span> {
    match tree {
        syn::UseTree::Path(p) => {
            (p.ident == "super" || p.ident == "self").then(|| p.ident.span())
        }
        syn::UseTree::Group(g) => g.items.iter().find_map(relative_use_root),
        _ => None,
    }
}
