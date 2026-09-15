mod check;
mod diagnostics;
mod generate;
mod lower;
mod model;
mod naming;

use amethystate_macros_core::MacroArgs;
use darling::{FromMeta, ast::NestedMeta};
use generate::generate_code;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::Span;
use quote::quote;
use syn::__private::TokenStream2;
use syn::{DeriveInput, parse_macro_input};

pub fn amethystate_impl(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr_args = match NestedMeta::parse_meta_list(args.into()) {
        Ok(v) => v,
        Err(e) => return darling::Error::from(e).write_errors().into(),
    };

    let macro_args = match MacroArgs::from_list(&attr_args) {
        Ok(v) => v,
        Err(e) => return e.write_errors().into(),
    };

    let input = parse_macro_input!(input as DeriveInput);

    let mut found = diagnostics::Diagnostics::new();

    if !input.generics.params.is_empty() {
        found.push(syn::Error::new_spanned(
            &input.generics,
            "a declared struct stands at a place in the store, and there is one of it: the \
             schema is submitted once, under one prefix, whatever it was written over. Hold \
             the varying part in a field, or declare the shapes you want separately",
        ));
    }

    let schema = match lower::schema(&input, &macro_args, &mut found) {
        Ok(schema) => schema,
        Err(e) => {
            found.push(e);
            return found.finish().unwrap_err().to_compile_error().into();
        }
    };

    check::schema(&schema, &mut found);

    if let Err(e) = found.finish() {
        return e.to_compile_error().into();
    }

    let expanded = generate_code(amethystate_crate_path(), &schema);

    proc_macro::TokenStream::from(expanded)
}

pub fn amethystate_crate_path() -> TokenStream2 {
    match crate_name("amethystate") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            quote!(::#ident)
        }
        _ => quote!(::amethystate),
    }
}
