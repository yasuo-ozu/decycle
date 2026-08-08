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
//! aliases are emitted as ordinary `use` items in the processed module, where they resolve exactly as
//! the original spelling did; the nesting then cannot change what they mean.
//!
//! Alias names are **deterministic** (they carry the module's own ident, not a random nonce), so
//! diagnostics that mention one are stable enough to match in a golden test.

use proc_macro2::{Group, Ident, Spacing, TokenStream, TokenTree};
use syn::spanned::Spanned;
use std::collections::HashSet;
use syn::visit_mut::VisitMut;
use syn::{
    GenericParam, Generics, Item, ItemImpl, Macro, Path, PathArguments, PathSegment, TraitBound,
    TypePath,
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
            let name = format!("__DecycleNat_{head}_{module_ident}");
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
    let mut lifted: Vec<(Ident, Path)> = Vec::new();
    for im in adopted.iter_mut() {
        LiftRelative {
            lifted: &mut lifted,
            cyclic_traits,
            module_ident,
        }
        .visit_item_impl_mut(im);
    }
    for (alias, path) in &lifted {
        items.push(syn::parse_quote! {
            #[allow(non_camel_case_types, unused_imports)]
            use #path as #alias;
        });
    }

    items
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

/// Replace a `super::…` / `self::…` rooted path with a single-segment alias, recording the original
/// so the caller can bind it. Generic arguments stay on the alias (`super::Wrap<T>` ⇒ `Alias<T>`),
/// since only the *path* needs resolving, not the instantiation.
///
/// Deliberately narrow — three kinds of path are left exactly as written:
/// - **anything naming a cyclic trait.** A relative trait reference is decycle's own signal, already
///   normalised by `strip_leading_self`; aliasing it to a one-segment name would make an
///   ordinary premise look like a cycle edge (or vice versa).
/// - **qualified paths** (`<B as self::Tr>::Assoc`). Their `qself.position` indexes into `segments`,
///   so collapsing the path to a single segment corrupts the syntax tree.
/// - **expressions.** Only types and trait bounds are visited; a body path is left to resolve
///   normally.
struct LiftRelative<'a> {
    lifted: &'a mut Vec<(Ident, Path)>,
    cyclic_traits: &'a HashSet<Ident>,
    module_ident: &'a Ident,
}

impl LiftRelative<'_> {
    fn lift(&mut self, path: &mut Path) {
        if path.leading_colon.is_some() || path.segments.len() < 2 {
            return;
        }
        let root = path.segments[0].ident.to_string();
        if root != "super" && root != "self" {
            return;
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
                        self.module_ident
                    ),
                    span,
                );
                self.lifted.push((a.clone(), bare));
                a
            }
        };
        *path = Path {
            leading_colon: None,
            segments: std::iter::once(PathSegment {
                ident: alias,
                arguments: args,
            })
            .collect(),
        };
    }

    fn names_cyclic_trait(&self, path: &Path) -> bool {
        path.segments
            .last()
            .is_some_and(|s| self.cyclic_traits.contains(&s.ident))
    }
}

impl VisitMut for LiftRelative<'_> {
    fn visit_type_path_mut(&mut self, tp: &mut TypePath) {
        // Depth-first, so an inner relative argument is lifted before its enclosing path.
        syn::visit_mut::visit_type_path_mut(self, tp);
        if tp.qself.is_some() || self.names_cyclic_trait(&tp.path) {
            return;
        }
        self.lift(&mut tp.path);
    }

    fn visit_trait_bound_mut(&mut self, tb: &mut TraitBound) {
        syn::visit_mut::visit_trait_bound_mut(self, tb);
        if self.names_cyclic_trait(&tb.path) {
            return;
        }
        self.lift(&mut tb.path);
    }
}
