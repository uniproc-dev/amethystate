use crate::amethystate::generate::{delete_tokens, path_literal, unreadable_tokens};
use crate::amethystate::model::{Field, Schema, Shape, StoredAs};
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};

/// How this field is stored, when its own type is not what stores it.
///
/// The write half is the throwaway `Serialize` serde derives at a
/// `serialize_with` field, made here because the struct serde would have put it
/// in is never encoded. The read half needs no wrapper: an erased deserializer
/// is a `serde::Deserializer`, so the function serde wants can be called with
/// it directly.
pub(crate) fn stored_as(crate_name: &TokenStream2, ty: &syn::Type, how: &StoredAs) -> TokenStream2 {
    let write = match how.write.as_ref() {
        Some(write) => quote! {
            Some({
                fn __ame_write(
                    value: &#ty,
                    then: &mut dyn FnMut(&dyn #crate_name::erased_serde::Serialize)
                        -> #crate_name::StorageResult<()>,
                ) -> #crate_name::StorageResult<()> {
                    struct Wrap<'a>(&'a #ty);

                    impl #crate_name::serde::Serialize for Wrap<'_> {
                        fn serialize<S: #crate_name::serde::Serializer>(
                            &self,
                            serializer: S,
                        ) -> ::std::result::Result<S::Ok, S::Error> {
                            #write(self.0, serializer)
                        }
                    }

                    then(&Wrap(value))
                }

                __ame_write as #crate_name::store::WriteAs<#ty>
            })
        },
        None => quote!(None),
    };

    let read = match how.read.as_ref() {
        Some(read) => quote! {
            Some({
                fn __ame_read<'de>(
                    deserializer: &mut dyn #crate_name::erased_serde::Deserializer<'de>,
                ) -> ::std::result::Result<#ty, #crate_name::erased_serde::Error> {
                    #read(deserializer)
                }

                __ame_read as #crate_name::store::ReadAs<#ty>
            })
        },
        None => quote!(None),
    };

    quote! {
        #crate_name::store::StoredAs { write: #write, read: #read }
    }
}

/// What each field is built from, in the order they were declared.
///
/// A rule the field wrote wins over one the struct wrote; where neither said,
/// the name of the argument the constructor takes is emitted, so the struct
/// holding this one decides at the call.
pub(crate) fn init_fields(crate_name: &TokenStream2, schema: &Schema) -> Vec<TokenStream2> {
    let is_root = schema.is_root();
    let on_unreadable = schema.rules.on_unreadable.as_ref().map(|at| at.value);
    let on_delete = schema.rules.on_delete.as_ref().map(|at| at.value);

    schema
        .fields
        .iter()
        .map(|field| {
            let unreadable = match field
                .rules
                .on_unreadable
                .as_ref()
                .map(|at| at.value)
                .or(on_unreadable)
            {
                Some(rule) => unreadable_tokens(crate_name, rule),
                None => quote!(__ame_on_unreadable),
            };

            let deleted = match field
                .rules
                .on_delete
                .as_ref()
                .map(|at| at.value)
                .or(on_delete)
            {
                Some(rule) => delete_tokens(crate_name, rule),
                None => quote!(__ame_on_delete),
            };

            init_field(crate_name, field, is_root, &unreadable, &deleted)
        })
        .collect::<Vec<_>>()
}

fn init_field(
    crate_name: &TokenStream2,
    field: &Field,
    is_root: bool,
    unreadable: &TokenStream2,
    deleted: &TokenStream2,
) -> TokenStream2 {
    let fname = &field.ident;
    let ty = &field.ty;
    let key_path = path_literal(crate_name, &field.stored.value);

    let at = if is_root {
        quote! { <Self as #crate_name::StateScope>::PATH.join(&#key_path) }
    } else {
        quote! { namespace.join(&#key_path) }
    };

    match &field.shape {
        Shape::Node { flattened } => {
            let under = match (is_root, flattened) {
                (true, false) => at,
                (true, true) => quote!(<Self as #crate_name::StateScope>::PATH.clone()),
                (false, false) => at,
                (false, true) => quote!(namespace.clone()),
            };

            quote! {
                #fname: ::std::sync::Arc::new(#ty::new_with_id_under(
                    store,
                    #under,
                    instance_id,
                    #unreadable,
                    #deleted
                )?)
            }
        }

        Shape::Map {
            key,
            value,
            default,
        } => {
            let def = default
                .clone()
                .unwrap_or_else(|| quote!(::std::collections::HashMap::new()));

            quote! {
                #fname: #crate_name::store::reactive_map_where::<#key, #value>(
                    store,
                    #at,
                    #def,
                    instance_id,
                    #unreadable,
                    #deleted
                )?
            }
        }

        Shape::Volatile { default } => {
            quote! { #fname: #crate_name::Field::new_volatile_with_id(#at, #default, instance_id) }
        }

        Shape::Leaf {
            default,
            stored_as: how,
        } => {
            let checked = match field.rules.check.as_ref() {
                Some(check) => {
                    let path = &check.value;
                    quote_spanned! {check.span=> .check(#path) }
                }
                None => quote! {},
            };

            let stored_as = match how {
                Some(how) => {
                    let how = stored_as(crate_name, ty, how);
                    quote! { .stored_as(#how) }
                }
                None => quote! {},
            };

            let rules = quote! {
                #crate_name::store::ReadRules::new()
                    .on_unreadable(#unreadable)
                    .on_delete(#deleted)
                    #checked
                    #stored_as
            };

            quote! { #fname: #crate_name::store::field_with_path_under(store, #at, #default, instance_id, #rules)? }
        }
    }
}
