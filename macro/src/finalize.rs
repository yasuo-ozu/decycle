use proc_macro2::{Span, TokenStream};
use proc_macro_error::*;
use syn::parse::{Parse, ParseStream};
use syn::*;
use template_quote::quote;

pub struct FinalizeArgs {
    pub working_list: Vec<Path>,
    pub traits: Vec<ItemTrait>,
    pub contents: Vec<ItemImpl>,
}

impl Parse for FinalizeArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let crate_version: LitStr = input.parse()?;
        let expected_version = env!("CARGO_PKG_VERSION");
        if crate_version.value() != expected_version {
            abort!(
                Span::call_site(),
                "version mismatch: expected '{}', got '{}'",
                expected_version,
                crate_version.value()
            )
        }

        let working_list_content;
        bracketed!(working_list_content in input);
        let mut working_list = Vec::new();
        while !working_list_content.is_empty() {
            working_list.push(working_list_content.parse()?);
            if !working_list_content.is_empty() {
                working_list_content.parse::<Token![,]>()?;
            }
        }

        let traits_content;
        braced!(traits_content in input);
        let mut traits = Vec::new();
        while !traits_content.is_empty() {
            traits.push(traits_content.parse()?);
            if !traits_content.is_empty() {
                traits_content.parse::<Token![,]>()?;
            }
        }

        let contents_content;
        braced!(contents_content in input);
        let mut contents = Vec::new();
        while !contents_content.is_empty() {
            contents.push(contents_content.parse()?);
            if !contents_content.is_empty() {
                contents_content.parse::<Token![,]>()?;
            }
        }

        Ok(FinalizeArgs {
            working_list,
            traits,
            contents,
        })
    }
}

impl template_quote::ToTokens for FinalizeArgs {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let crate_version = env!("CARGO_PKG_VERSION");
        let working_list = &self.working_list;
        let traits = &self.traits;
        let contents = &self.contents;

        tokens.extend(quote! {
            #crate_version
            [ #(#working_list),* ]
            { #(#traits),* }
            { #(#contents),* }
        });
    }
}

pub fn finalize(args: FinalizeArgs) -> TokenStream {
    quote!()
}
