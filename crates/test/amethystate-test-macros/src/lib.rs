//! One statement, asked of the engines it is about.
//!
//! A test that takes a `Backend` is written once and run once per engine. The
//! `#[cfg]` for each is emitted here rather than typed by the author, which is
//! the point: a suite whose whole purpose is that engines disagree spent a
//! long time asking one of them, because the ladder that picked them was
//! written by hand in seven places and each one could be short.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, ItemFn, Token, parse_macro_input};

/// Every engine, by the feature that enables it and the variant it is.
const ENGINES: &[(&str, &str)] = &[
    ("redb", "Redb"),
    ("sqlite", "Sqlite"),
    ("json", "Json"),
    ("toml", "Toml"),
    ("ron", "Ron"),
];

/// The engines that keep the whole store in one document a person could open
/// in an editor.
const TEXT: &[&str] = &["json", "toml", "ron"];

enum Which {
    All,
    Text,
    Only(Vec<String>),
}

impl Parse for Which {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let first: Ident = input.parse()?;

        match first.to_string().as_str() {
            "all" if input.is_empty() => Ok(Which::All),
            "text" if input.is_empty() => Ok(Which::Text),
            _ => {
                let mut named = vec![first.to_string()];
                while input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                    if input.is_empty() {
                        break;
                    }
                    let next: Ident = input.parse()?;
                    named.push(next.to_string());
                }

                for name in &named {
                    if !ENGINES.iter().any(|(_, variant)| variant == name) {
                        return Err(syn::Error::new(
                            first.span(),
                            format!(
                                "`{name}` is not an engine. Say `all`, `text`, or one or more \
                                 of {}",
                                ENGINES
                                    .iter()
                                    .map(|(_, variant)| *variant)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        ));
                    }
                }

                Ok(Which::Only(named))
            }
        }
    }
}

impl Which {
    fn wanted(&self) -> Vec<(&'static str, &'static str)> {
        ENGINES
            .iter()
            .copied()
            .filter(|(feature, variant)| match self {
                Which::All => true,
                Which::Text => TEXT.contains(feature),
                Which::Only(named) => named.iter().any(|name| name == variant),
            })
            .collect()
    }
}

/// Runs this test once for each engine it names, with that engine as its
/// argument.
///
/// ```ignore
/// #[backends(all)]
/// fn a_leaf_answers_with_itself(backend: Backend) { .. }
/// ```
///
/// becomes `a_leaf_answers_with_itself::redb`, `::sqlite`, `::json`,
/// `::toml` and `::ron`, each behind the feature that enables it. The body is
/// written once and called by each, so a failure names the engine that failed
/// and the others still run.
///
/// `text` is the three document engines. Naming variants - `#[backends(Redb)]`,
/// `#[backends(Json, Toml)]` - is for a statement true of those alone.
#[proc_macro_attribute]
pub fn backends(attr: TokenStream, item: TokenStream) -> TokenStream {
    let which = parse_macro_input!(attr as Which);
    let body = parse_macro_input!(item as ItemFn);

    if body.sig.inputs.len() != 1 {
        return syn::Error::new_spanned(
            &body.sig,
            "a test written for several engines takes the engine it is running on, \
             and nothing else",
        )
        .to_compile_error()
        .into();
    }

    let mut body = body;

    // `#[ignore]` and `#[should_panic]` are about running a test, and the
    // function the author wrote is no longer one - it is the body the cases
    // call. Left where they were written they would be dropped in silence, and
    // a test parked as known-broken would quietly start running again.
    let (on_the_test, on_the_body): (Vec<_>, Vec<_>) = body
        .attrs
        .into_iter()
        .partition(|attr| attr.path().is_ident("ignore") || attr.path().is_ident("should_panic"));
    body.attrs = on_the_body;

    let name = &body.sig.ident;
    let returns = &body.sig.output;

    let cases = which.wanted().into_iter().map(|(feature, variant)| {
        let on_the_test = &on_the_test;
        let case = format_ident!("{}", feature);
        let variant = format_ident!("{}", variant);
        quote! {
            #[cfg(feature = #feature)]
            #[test]
            #(#on_the_test)*
            fn #case() #returns {
                super::#name(::amethystate::store::builder::Backend::#variant)
            }
        }
    });

    quote! {
        #body

        #[allow(non_snake_case)]
        mod #name {
            use super::*;
            #(#cases)*
        }
    }
    .into()
}
