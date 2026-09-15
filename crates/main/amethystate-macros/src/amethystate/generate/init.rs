use crate::amethystate::generate::{
    delete_tokens, entries_tokens, path_literal, unreadable_tokens,
};
use crate::amethystate::model::{Field, Schema, Shape, StoredAs};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;

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
    let entries = schema.rules.unreadable_entries.as_ref().map(|at| at.value);

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

            let entries = match field
                .rules
                .unreadable_entries
                .as_ref()
                .map(|at| at.value)
                .or(entries)
            {
                Some(rule) => entries_tokens(crate_name, rule),
                None => quote!(__ame_unreadable_entries),
            };

            init_field(crate_name, field, is_root, &unreadable, &deleted, &entries)
        })
        .collect::<Vec<_>>()
}

fn init_field(
    crate_name: &TokenStream2,
    field: &Field,
    is_root: bool,
    unreadable: &TokenStream2,
    deleted: &TokenStream2,
    entries: &TokenStream2,
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
                    #deleted,
                    #entries
                )?)
            }
        }

        Shape::Volatile { default } => {
            let named = fname.to_string();
            let not_a_map = is_not_a_map(
                crate_name,
                ty,
                ty.span(),
                &format!(
                    "`{named}` is a `volatile` map, and a `ReactiveMap` is entries under a path: \
                     it is built against the store and has nowhere else to keep them. A map this \
                     process holds and never stores is a plain field over `HashMap`"
                ),
            );

            quote! {
                #fname: {
                    #not_a_map
                    #crate_name::Field::new_volatile_with_id(#at, #default, instance_id)
                }
            }
        }

        Shape::Stored {
            default,
            stored_as: how,
        } => {
            let named = fname.to_string();

            let seed = super::seed_tokens(default);

            let check = match field.rules.check.as_ref() {
                Some(check) => {
                    let path = &check.value;
                    quote_spanned! {check.span=>
                        ::core::option::Option::Some(#path as #crate_name::store::Check<#ty>)
                    }
                }
                None => quote! { ::core::option::Option::None },
            };

            let stored_as = match how {
                Some(how) => stored_as(crate_name, ty, how),
                None => quote! { #crate_name::store::StoredAs::default() },
            };

            let mut refusals = Vec::new();

            if let Some(said) = &field.rules.check {
                refusals.push(is_not_a_map(
                    crate_name,
                    ty,
                    said.span,
                    &format!(
                        "`{named}` is a map, and its entries are data rather than declared paths: \
                         one bad entry is no reason to withhold the struct, so a map wants \
                         dropping and reporting rather than this"
                    ),
                ));
            }

            if let Some(said) = &field.rules.on_unreadable {
                refusals.push(is_not_a_map(
                    crate_name,
                    ty,
                    said.span,
                    "a map has no default to stand in for an entry - what it declares seeds a \
                     store holding none - so `on_unreadable` decides nothing here. \
                     `unreadable_entries` is the answer about its entries",
                ));
            }

            if let Some(said) = &field.rules.unreadable_entries {
                refusals.push(is_a_map(
                    crate_name,
                    ty,
                    said.span,
                    "only a map has entries. This field holds one value, and what it does about a \
                     value it cannot read is `on_unreadable`",
                ));
            }

            quote! {
                #fname: {
                    #(#refusals)*
                    <#ty as #crate_name::shape::Kind>::build(
                        store,
                        #at,
                        #seed,
                        instance_id,
                        #crate_name::shape::KindRules {
                            on_unreadable: #unreadable,
                            on_delete: #deleted,
                            unreadable_entries: #entries,
                            check: #check,
                            stored_as: #stored_as,
                        },
                    )?
                }
            }
        }
    }
}

/// A compile-time refusal of `ty` being a map, said in `message`.
///
/// The macro cannot tell a map from a value while it expands - that is the
/// compiler's to answer, after - so a rule that means nothing on a map is
/// refused where the answer exists.
fn is_not_a_map(
    crate_name: &TokenStream2,
    ty: &syn::Type,
    span: Span,
    message: &str,
) -> TokenStream2 {
    quote_spanned! {span=>
        const _: () = {
            #[allow(unused_imports)]
            use #crate_name::shape::AnyShape as _;
            assert!(
                !<#crate_name::shape::Probe<#ty>>::ROLE
                    .same(#crate_name::migration::fields::Role::Map),
                #message
            );
        };
    }
}

/// The same refusal the other way round: `ty` has to be a map.
fn is_a_map(crate_name: &TokenStream2, ty: &syn::Type, span: Span, message: &str) -> TokenStream2 {
    quote_spanned! {span=>
        const _: () = {
            #[allow(unused_imports)]
            use #crate_name::shape::AnyShape as _;
            assert!(
                <#crate_name::shape::Probe<#ty>>::ROLE
                    .same(#crate_name::migration::fields::Role::Map),
                #message
            );
        };
    }
}
