mod generate;

use amethystate_macros_core::{MacroArgs, StoreFieldEntry};
use darling::{FromField, FromMeta, ast::NestedMeta};
use generate::generate_code;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::Span;
use quote::quote;
use syn::__private::TokenStream2;
use syn::spanned::Spanned;
use syn::{Data, DataStruct, DeriveInput, Fields, parse_macro_input};

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

    let prefix = if macro_args.as_root {
        Some(generate::ROOT.to_string())
    } else {
        match &macro_args.prefix {
            Some(prefix) => {
                if let Err(message) = generate::check_written_path(
                    "prefix",
                    prefix,
                    "; write `as_root` for a struct whose fields sit at the top of the store",
                ) {
                    return syn::Error::new(prefix.span(), message)
                        .to_compile_error()
                        .into();
                }
                Some(prefix.as_ref().clone())
            }
            None => None,
        }
    };

    let on_struct = match generate::read_policy(macro_args.on_unreadable.as_ref()) {
        Ok(policy) => policy,
        Err(e) => return e.to_compile_error().into(),
    };

    if let Err(e) = generate::delete_policy(macro_args.on_delete.as_ref()) {
        return e.to_compile_error().into();
    }

    if let Some(check) = &macro_args.check
        && macro_args.mode.as_deref() == Some("persistent")
    {
        return syn::Error::new(
            check.span(),
            "a struct's check is handed the struct, and `mode = \"persistent\"` builds none - what `load_with` returns is the stored data. Declare the check on the fields, which are read either way, or take the mode off",
        )
        .to_compile_error()
        .into();
    }

    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;
    let struct_vis = &input.vis;
    let attrs = &input.attrs;
    let amethystate = amethystate_crate_path();

    let named_fields = match &input.data {
        Data::Struct(DataStruct {
            fields: Fields::Named(f),
            ..
        }) => &f.named,
        _ => {
            return darling::Error::custom(
                "amethystate can only be used on structs with named fields",
            )
            .with_span(struct_name)
            .write_errors()
            .into();
        }
    };

    let mut entries = Vec::new();
    for field in named_fields {
        let entry = match StoreFieldEntry::from_field(field) {
            Ok(v) => v,
            Err(e) => return e.write_errors().into(),
        };

        if let Some(key) = &entry.key
            && let Err(message) = generate::check_written_path("key", key, "")
        {
            return syn::Error::new(key.span(), message)
                .to_compile_error()
                .into();
        }

        if entry.nested && generate::names_type(&entry.ty, struct_name) {
            return syn::Error::new(
                entry.ty.span(),
                format!(
                    "`{struct_name}` would build another `{struct_name}` to build itself, and never stop. A node that holds one of its own kind has to be able to stop - `Option<Box<_>>` at `None`, a collection at empty - and neither is built the way a `nested` field is"
                ),
            )
            .to_compile_error()
            .into();
        }

        if let Some(check) = &entry.check {
            let named = entry.stored_name();

            let refusal = if entry.volatile {
                Some(format!(
                    "`{named}` is volatile, so nothing arrives from the store for a check to judge. A value this process holds and never stores is the interceptor's business"
                ))
            } else if entry.nested {
                Some(format!(
                    "`{named}` is a nested struct, and a check on one belongs on the struct itself - `#[amethystate(check = ..)]` there is handed every field of it at once, which is what a rule about a struct needs"
                ))
            } else if entry.get_map_types().is_some() {
                Some(format!(
                    "`{named}` is a map, and its entries are data rather than declared paths: one bad entry is no reason to withhold the struct, so a map wants dropping and reporting rather than this"
                ))
            } else {
                None
            };

            if let Some(message) = refusal {
                return syn::Error::new(check.span(), message).to_compile_error().into();
            }
        }

        let on_field = match generate::read_policy(entry.on_unreadable.as_ref()) {
            Ok(policy) => policy,
            Err(e) => return e.to_compile_error().into(),
        };

        if let Err(e) = generate::delete_policy(entry.on_delete.as_ref()) {
            return e.to_compile_error().into();
        }

        if on_struct.as_deref() == Some("Refuse")
            && on_field.as_deref() == Some("UseDefault")
            && let Some(written) = &entry.on_unreadable
        {
            let named = entry.stored_name();
            return syn::Error::new(
                written.span(),
                format!(
                    "`{struct_name}` declares `on_unreadable = Refuse`, so `{named}` cannot ask for `UseDefault`. A field may demand more than the struct promised and never less: drop this to inherit the struct's rule, or move `UseDefault` up to the struct and write `Refuse` on the fields that must be readable"
                ),
            )
            .to_compile_error()
            .into();
        }

        entries.push(entry);
    }

    let expanded = generate_code(
        amethystate,
        struct_vis,
        struct_name,
        attrs,
        prefix,
        &entries,
        macro_args,
    );

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
