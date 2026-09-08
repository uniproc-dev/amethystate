use crate::amethystate::generate::{path_parts, unreadable_tokens};
use crate::amethystate::model::{Field, Mode, Schema, Shape};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;

/// The stored type of one field, as it appears in the reactive struct.
pub(crate) fn field_type(crate_name: &TokenStream2, field: &Field) -> TokenStream2 {
    let ty = &field.ty;

    match &field.shape {
        Shape::Node { .. } => quote! { ::std::sync::Arc<#ty> },
        Shape::Map { key, value, .. } => quote! { #crate_name::ReactiveMap<#key, #value> },
        Shape::Leaf { .. } | Shape::Volatile { .. } => quote! { #crate_name::Field<#ty> },
    }
}

pub(crate) fn struct_fields<'a>(
    crate_name: &'a TokenStream2,
    fields: &'a [Field],
) -> impl Iterator<Item = TokenStream2> + 'a {
    fields.iter().map(move |field| {
        let fname = &field.ident;
        let fvis = &field.vis;
        let ty = field_type(crate_name, field);
        let carried = &field.forwarded;

        quote! { #(#carried)* #fvis #fname: #ty }
    })
}

/// A getter per field, handing back a clone of what the struct holds.
pub(crate) fn methods(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let each = schema.fields.iter().map(|field| {
        let fname = &field.ident;
        let held = field_type(crate_name, field);
        let carried = &field.forwarded;

        quote! {
            #(#carried)*
            pub fn #fname(&self) -> #held {
                self.#fname.clone()
            }
        }
    });

    quote! { #(#each)* }
}

/// The types this struct's constructor always constructs in turn.
///
/// A `nested` field is built unconditionally, so those are the edges a cycle
/// can run along. Nothing else is: a map recursing through its value type
/// decodes those values rather than constructing them.
fn construction_edges(crate_name: &TokenStream2, fields: &[Field]) -> Vec<TokenStream2> {
    fields
        .iter()
        .filter(|field| matches!(field.shape, Shape::Node { .. }))
        .map(|field| {
            let ty = &field.ty;
            quote_spanned! {ty.span()=>
                let _: () = <#ty as #crate_name::AmeStateNode>::CONSTRUCTION_TERMINATES;
            }
        })
        .collect()
}

pub(crate) fn node_impl(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if schema.mode == Mode::Persistent {
        return quote! {};
    }

    let name = &schema.name;
    let edges = construction_edges(crate_name, &schema.fields);
    let terminates = quote! {
        const CONSTRUCTION_TERMINATES: () = { #(#edges)* };
    };
    let force = quote! {
        const _: () = <#name as #crate_name::AmeStateNode>::CONSTRUCTION_TERMINATES;
    };

    if schema.is_root() {
        quote! {
            impl #crate_name::AmeStateNode for #name {
                #terminates

                fn new_node(store: &#crate_name::Store, _path: &#crate_name::store::StorePath) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                    Self::new_with(store)
                }

                fn new_node_with_id(store: &#crate_name::Store, _path: &#crate_name::store::StorePath, instance_id: #crate_name::uuid::Uuid) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                    Self::new_with_id(store, instance_id)
                }
            }

            #force
        }
    } else {
        quote! {
            impl #crate_name::AmeStateNode for #name {
                #terminates

                fn new_node(store: &#crate_name::Store, path: &#crate_name::store::StorePath) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                    Self::new(store, path)
                }

                fn new_node_with_id(store: &#crate_name::Store, path: &#crate_name::store::StorePath, instance_id: #crate_name::uuid::Uuid) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                    Self::new_with_id(store, path, instance_id)
                }
            }

            #force
        }
    }
}

/// Where this struct sits, as a constant on the type.
///
/// A struct meant to be embedded has none: it sits wherever its holder puts
/// it, which is a value rather than a constant.
pub(crate) fn scope(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let Some(placement) = &schema.prefix else {
        return quote! {};
    };

    let name = &schema.name;
    let written = placement.path();
    let (segments, joined) = path_parts(&written);

    quote! {
        impl #crate_name::StateScope for #name {
            const PATH: #crate_name::store::StorePath =
                #crate_name::store::StorePath::from_static(&[#(#segments),*], #joined);
            const KEY: &'static str = #joined;
        }
    }
}

/// How a struct with a place of its own is loaded and watched by callers that
/// know it only as a slice of the store.
///
/// A struct without a place has no `load_slice`: there is nowhere to load it
/// from until something says where.
pub(crate) fn slice_impl(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if !schema.is_root() {
        return quote! {};
    }

    let name = &schema.name;
    let mode = schema.mode;

    let load = match mode {
        Mode::Persistent => quote! { Self::load_with(store) },
        _ => quote! { Self::new_with(store) },
    };

    let subs = if matches!(mode, Mode::Reactive | Mode::Both) {
        quote! {
            fn subscribe_all<F>(&self, callback: F) -> #crate_name::ReactiveScope
            where
                F: Fn() + Send + Sync + 'static,
            {
                self.subscribe_all(callback)
            }

            fn subscribe_all_external<F>(&self, callback: F) -> #crate_name::ReactiveScope
            where
                F: Fn() + Send + Sync + 'static,
            {
                self.subscribe_all_external(callback)
            }
        }
    } else {
        quote! {
            fn subscribe_all<F>(&self, _callback: F) -> #crate_name::ReactiveScope
            where
                F: Fn() + Send + Sync + 'static,
            {
                #crate_name::ReactiveScope::new()
            }

            fn subscribe_all_external<F>(&self, _callback: F) -> #crate_name::ReactiveScope
            where
                F: Fn() + Send + Sync + 'static,
            {
                #crate_name::ReactiveScope::new()
            }
        }
    };

    quote! {
        impl #crate_name::AmeStateSlice for #name {
            fn load_slice(store: &#crate_name::Store) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                #load
            }

            #subs
        }
    }
}

/// `new()` against the store this process installed globally.
pub(crate) fn global_new(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if !schema.is_root() || schema.mode == Mode::Persistent {
        return quote! {};
    }

    let name = &schema.name;

    quote! {
        impl #name {
            pub fn new() -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                let store = #crate_name::global_store();
                Self::new_with(&store)
            }
        }
    }
}

/// Marks the fields a struct's own check named, so each of them answers
/// `try_get` with what the check said.
///
/// A nested field is marked all the way down: what failed is a relationship
/// the holder declared, and nothing inside the nested struct can be told apart
/// by it.
pub(crate) fn refused_marker(schema: &Schema) -> TokenStream2 {
    let marks = schema
        .fields
        .iter()
        .filter(|field| !matches!(field.shape, Shape::Map { .. }))
        .map(|field| {
            let fname = &field.ident;
            let named = fname.to_string();

            let mark = match field.shape {
                Shape::Node { .. } => {
                    quote! { self.#fname.__ame_refused(::core::option::Option::None, why); }
                }
                _ => quote! { self.#fname.__ame_refused(why); },
            };

            quote! {
                if fields.is_none_or(|named| named.contains(&#named)) {
                    #mark
                }
            }
        });

    quote! {
        #[doc(hidden)]
        pub fn __ame_refused(&self, fields: ::core::option::Option<&[&str]>, why: &str) {
            let _ = (&fields, why);
            #(#marks)*
        }
    }
}

/// What a struct's own check does when it refuses, in the constructor that
/// has just built every field.
fn struct_check(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let Some(check) = schema.rules.check.as_ref().map(|at| &at.value) else {
        return quote! {};
    };

    let rule = match schema.rules.on_unreadable.as_ref().map(|at| at.value) {
        Some(rule) => unreadable_tokens(crate_name, rule),
        None => quote!(__ame_on_unreadable),
    };

    let where_it_is = if schema.is_root() {
        quote! { <Self as #crate_name::StateScope>::PATH.clone() }
    } else {
        quote! { namespace.clone() }
    };

    quote_spanned! {check.span()=>
        if let ::core::result::Result::Err(__ame_invalid) = #check(&result.__ame_to_data(), store.context()) {
            match #rule {
                #crate_name::store::OnUnreadable::Refuse => {
                    return ::core::result::Result::Err(
                        #crate_name::store::OpenStruct::Refused {
                            at: #where_it_is,
                            said: ::std::sync::Arc::from(__ame_invalid.reason()),
                        }
                    );
                }
                #crate_name::store::OnUnreadable::UseDefault => {
                    result.__ame_refused(__ame_invalid.fields(), __ame_invalid.reason());
                }
            }
        }
    }
}

pub(crate) fn constructor(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let checked = struct_check(crate_name, schema);
    let init_fields = super::init::init_fields(crate_name, schema);
    let data_struct_name = format_ident!("{}_Data", schema.name);
    let struct_label = schema.name.to_string();

    if schema.is_root() {
        quote! {
            pub fn new_with(store: &#crate_name::Store) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                Self::new_with_id(store, #crate_name::uuid::Uuid::new_v4())
            }

            pub fn new_with_id(store: &#crate_name::Store, instance_id: #crate_name::uuid::Uuid) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                let __ame_fallbacks = store.fallbacks();
                Self::new_with_id_under(
                    store,
                    instance_id,
                    __ame_fallbacks.on_unreadable,
                    __ame_fallbacks.on_delete,
                    __ame_fallbacks.unreadable_entries,
                )
            }

            /// The same, told what the struct holding this one decided about a
            /// value it cannot read, a key removed under it, and an entry of a
            /// map that will not read.
            ///
            /// Whatever this struct declared for itself wins; these are what a
            /// field falls back to when neither it nor this struct said.
            pub fn new_with_id_under(
                store: &#crate_name::Store,
                instance_id: #crate_name::uuid::Uuid,
                __ame_on_unreadable: #crate_name::store::OnUnreadable,
                __ame_on_delete: #crate_name::store::OnDelete,
                __ame_unreadable_entries: #crate_name::store::UnreadableEntries,
            ) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                use #crate_name::{StoreBackend, StoreExt};
                let __amethystate_guard = #crate_name::store::instances::InstanceGuard::new(
                    instance_id,
                    ::std::any::type_name::<Self>(),
                );
                let result = Self {
                    __amethystate_instance_id: __amethystate_guard,
                    __amethystate_at: <Self as #crate_name::StateScope>::PATH.clone(),
                    #(#init_fields,)*
                };
                #checked
                store.mark_initialized(&<Self as #crate_name::StateScope>::PATH)?;
                Ok(result)
            }
        }
    } else {
        quote! {
            pub fn new(
                store: &#crate_name::Store,
                namespace: impl #crate_name::store::IntoStorePath,
            ) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                Self::new_with_id(store, namespace, #crate_name::uuid::Uuid::new_v4())
            }

            pub fn new_with_id(
                store: &#crate_name::Store,
                namespace: impl #crate_name::store::IntoStorePath,
                instance_id: #crate_name::uuid::Uuid,
            ) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                use #crate_name::StoreBackend;

                let __ame_fallbacks = store.fallbacks();
                let namespace = namespace.into_store_path()?;

                let built = Self::new_with_id_under(
                    store,
                    namespace.clone(),
                    instance_id,
                    __ame_fallbacks.on_unreadable,
                    __ame_fallbacks.on_delete,
                    __ame_fallbacks.unreadable_entries,
                )?;

                store.record_schema(
                    &namespace,
                    &#crate_name::store::meta::SchemaSnapshot {
                        version: <#data_struct_name as #crate_name::migration::fields::AmeStateFields>::VERSION,
                        struct_name: ::core::option::Option::Some(
                            <::std::string::String as ::core::convert::From<&str>>::from(#struct_label),
                        ),
                        fields: <#data_struct_name as #crate_name::migration::fields::AmeStateFields>::FIELDS
                            .iter()
                            .map(#crate_name::store::meta::StoredFieldEntry::from)
                            .collect(),
                    },
                )?;

                Ok(built)
            }

            /// The same, told what the struct holding this one decided about a
            /// value it cannot read, a key removed under it, and an entry of a
            /// map that will not read.
            ///
            /// Whatever this struct declared for itself wins; these are what a
            /// field falls back to when neither it nor this struct said.
            pub fn new_with_id_under(
                store: &#crate_name::Store,
                namespace: impl #crate_name::store::IntoStorePath,
                instance_id: #crate_name::uuid::Uuid,
                __ame_on_unreadable: #crate_name::store::OnUnreadable,
                __ame_on_delete: #crate_name::store::OnDelete,
                __ame_unreadable_entries: #crate_name::store::UnreadableEntries,
            ) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                use #crate_name::{StoreBackend, StoreExt};
                let namespace = namespace.into_store_path()?;
                let __amethystate_guard = #crate_name::store::instances::InstanceGuard::new(
                    instance_id,
                    ::std::any::type_name::<Self>(),
                );
                let result = Self {
                    __amethystate_instance_id: __amethystate_guard,
                    __amethystate_at: namespace.clone(),
                    #(#init_fields,)*
                };
                #checked
                store.mark_initialized(&namespace)?;
                Ok(result)
            }
        }
    }
}
