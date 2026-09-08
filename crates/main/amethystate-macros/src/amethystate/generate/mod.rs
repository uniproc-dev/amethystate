mod accessors;
mod data;
mod export;
mod init;
mod introspect;
mod native;
mod policy;
mod reactive;
mod wasm;

use super::model::{OnDelete, OnUnreadable, Schema, Target, UnreadableEntries};
use proc_macro2::{Delimiter, TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, Token};

/// A written path, as a value the compiler has already checked.
///
/// The `const` block is what makes the check a check: `from_static` verifies
/// its two halves in a `const fn`, which only runs at compile time where the
/// call is in a const context. Emitted bare, a path built inside a function
/// body would pay for that walk on every call and fail at first use instead of
/// at the attribute.
pub(crate) fn path_literal(crate_name: &TokenStream2, dotted: &str) -> TokenStream2 {
    let (segments, joined) = path_parts(dotted);

    quote! {
        const { #crate_name::store::StorePath::from_static(&[#(#segments),*], #joined) }
    }
}

/// The same path as [`path_literal`], for the places a declaration carries it:
/// a `const` item, and a `static` whose other fields are filled in around it.
///
/// Not wrapped in a `const` block, because it is already written into one.
pub(crate) fn static_path_literal(crate_name: &TokenStream2, dotted: &str) -> TokenStream2 {
    let (segments, joined) = path_parts(dotted);

    quote! {
        #crate_name::store::StaticPath::new(&[#(#segments),*], #joined)
    }
}

pub(crate) const SEPARATOR: char = '.';
pub(crate) const ESCAPE: char = '\\';
pub(crate) const ROOT: &str = ".";

/// The levels a written path names, and the joined form `StorePath` keeps
/// beside them.
///
/// Nothing here knows how a path is joined. Levels come from splitting on the
/// separator, so no level holds one, and the only character the join would have
/// escaped is the escape itself - which [`check_written_path`] refuses. What is
/// left is the identity, so the source string is the joined form, and
/// `StorePath::from_static` checks that claim in a const context rather than
/// taking it.
pub(crate) fn path_parts(dotted: &str) -> (Vec<&str>, String) {
    if dotted.is_empty() || dotted == ROOT {
        return (Vec::new(), String::new());
    }

    let segments: Vec<&str> = dotted.split(SEPARATOR).collect();

    (segments, dotted.to_string())
}

/// Whether `ty` is written as `name`, however it is qualified.
pub(crate) fn names_type(ty: &syn::Type, name: &Ident) -> bool {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => path
            .path
            .segments
            .last()
            .is_some_and(|last| &last.ident == name && last.arguments.is_empty()),
        _ => false,
    }
}

pub(crate) fn check_written_path(what: &str, written: &str, root_hint: &str) -> Result<(), String> {
    if written.is_empty() {
        return Err(format!("an empty {what} names no level{root_hint}"));
    }

    if written == ROOT {
        return Err(format!("a {what} of `{ROOT}` names no level{root_hint}"));
    }

    let levels: Vec<&str> = written.split(SEPARATOR).collect();
    let last = levels.len() - 1;

    for (at, level) in levels.iter().enumerate() {
        if level.is_empty() {
            return Err(match at {
                0 => {
                    format!("the {what} starts with `{SEPARATOR}`, so its first level has no name")
                }
                _ if at == last => {
                    format!("the {what} ends with `{SEPARATOR}`, so its last level has no name")
                }
                _ => {
                    format!("the {what} has two `{SEPARATOR}` in a row, with no level between them")
                }
            });
        }

        if level.contains(ESCAPE) {
            return Err(format!(
                "a {what} level cannot hold `{ESCAPE}`, which a path escapes"
            ));
        }
    }

    Ok(())
}

/// The variant of `OnUnreadable` a rule names, as the generated code writes it.
pub(crate) fn unreadable_tokens(crate_name: &TokenStream2, rule: OnUnreadable) -> TokenStream2 {
    match rule {
        OnUnreadable::Refuse => quote!(#crate_name::store::OnUnreadable::Refuse),
        OnUnreadable::UseDefault => quote!(#crate_name::store::OnUnreadable::UseDefault),
    }
}

pub(crate) fn delete_tokens(crate_name: &TokenStream2, rule: OnDelete) -> TokenStream2 {
    match rule {
        OnDelete::UseDefault => quote!(#crate_name::store::OnDelete::UseDefault),
        OnDelete::Keep => quote!(#crate_name::store::OnDelete::Keep),
    }
}

pub(crate) fn entries_tokens(crate_name: &TokenStream2, rule: UnreadableEntries) -> TokenStream2 {
    match rule {
        UnreadableEntries::Refuse => quote!(#crate_name::store::UnreadableEntries::Refuse),
        UnreadableEntries::Skip => quote!(#crate_name::store::UnreadableEntries::Skip),
    }
}

/// Where the generated code runs, which decides what is generated at all.
///
/// The two targets share a declaration and nothing else: one builds against a
/// store this process holds, the other against one on the other side of a
/// Tauri command.
pub fn generate_code(crate_name: TokenStream2, schema: &Schema) -> TokenStream2 {
    match schema.target {
        Target::Native => native::generate(&crate_name, schema),
        Target::TauriWasm => wasm::generate(&crate_name, schema),
    }
}

struct MapEntry {
    key: Expr,
    _colon: Token![:],
    value: Expr,
}

impl Parse for MapEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(MapEntry {
            key: input.parse()?,
            _colon: input.parse()?,
            value: input.parse()?,
        })
    }
}

pub(crate) fn parse_default(tokens: &TokenStream2) -> TokenStream2 {
    let mut iter = tokens.clone().into_iter();

    match iter.next() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket => {
            let content = g.stream();
            quote! { vec![#content] }
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            let content = g.stream();

            if content.is_empty() {
                return quote! { ::std::collections::HashMap::default() };
            }

            let parser = Punctuated::<MapEntry, Token![,]>::parse_terminated;
            if let Ok(pairs) = parser.parse2(content)
                && !pairs.is_empty()
            {
                let inserts = pairs.iter().map(|pair| {
                    let k = &pair.key;
                    let v = &pair.value;
                    quote! { __map.insert(::std::convert::Into::into(#k), #v); }
                });

                return quote! {
                    {
                        let mut __map = ::std::collections::HashMap::default();
                        #( #inserts )*
                        __map
                    }
                };
            }

            tokens.clone()
        }
        _ => tokens.clone(),
    }
}
