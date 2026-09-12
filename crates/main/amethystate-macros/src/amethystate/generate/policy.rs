//! What a struct promised about a value it cannot read, written where the
//! store and the structs holding it can both see it.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use super::{delete_tokens, entries_tokens, unreadable_tokens};
use crate::amethystate::model::{OnUnreadable, Schema, Shape};

/// The `DeclaredPolicy` impl, and the assertions that keep a nested struct
/// from loosening what its holder promised.
pub(crate) fn declared(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let name = &schema.name;
    let on_unreadable = schema.rules.on_unreadable.as_ref().map(|at| at.value);

    let declared_unreadable = match on_unreadable {
        Some(rule) => {
            let rule = unreadable_tokens(crate_name, rule);
            quote!(::core::option::Option::Some(#rule))
        }
        None => quote!(::core::option::Option::None),
    };

    let declared_delete = match schema.rules.on_delete.as_ref().map(|at| at.value) {
        Some(rule) => {
            let rule = delete_tokens(crate_name, rule);
            quote!(::core::option::Option::Some(#rule))
        }
        None => quote!(::core::option::Option::None),
    };

    let declared_entries = match schema.rules.unreadable_entries.as_ref().map(|at| at.value) {
        Some(rule) => {
            let rule = entries_tokens(crate_name, rule);
            quote!(::core::option::Option::Some(#rule))
        }
        None => quote!(::core::option::Option::None),
    };

    let held_to_it = holders(crate_name, schema, on_unreadable);

    quote! {
        impl #crate_name::store::DeclaredPolicy for #name {
            const ON_UNREADABLE: ::core::option::Option<#crate_name::store::OnUnreadable> =
                #declared_unreadable;
            const ON_DELETE: ::core::option::Option<#crate_name::store::OnDelete> =
                #declared_delete;
            const UNREADABLE_ENTRIES:
                ::core::option::Option<#crate_name::store::UnreadableEntries> =
                #declared_entries;
        }

        #(#held_to_it)*
    }
}

/// A nested field under `Refuse` cannot be a struct that declared `UseDefault`.
///
/// The nested struct's own declaration is a `const` on it, so the two are
/// compared where the field is written rather than where the struct is.
fn holders(
    crate_name: &TokenStream2,
    schema: &Schema,
    on_unreadable: Option<OnUnreadable>,
) -> Vec<TokenStream2> {
    let name = &schema.name;

    schema
        .fields
        .iter()
        .filter(|field| matches!(field.shape, Shape::Node { .. }))
        .filter(|field| {
            field
                .rules
                .on_unreadable
                .as_ref()
                .map(|at| at.value)
                .or(on_unreadable)
                == Some(OnUnreadable::Refuse)
        })
        .map(|field| {
            let ty = &field.ty;
            let held = &field.stored.value;
            let written = quote!(#ty).to_string();
            let complaint = syn::LitStr::new(
                &format!(
                    "`{name}` declares `on_unreadable = Refuse` over `{held}`, and `{written}` declares `UseDefault` for itself. A struct may demand more than the one holding it and never less: take `UseDefault` off `{written}`, so it inherits, or stop promising `Refuse` here"
                ),
                proc_macro2::Span::call_site(),
            );

            quote! {
                const _: () = assert!(
                    !matches!(
                        <#ty as #crate_name::store::DeclaredPolicy>::ON_UNREADABLE,
                        ::core::option::Option::Some(#crate_name::store::OnUnreadable::UseDefault)
                    ),
                    #complaint
                );
            }
        })
        .collect()
}
