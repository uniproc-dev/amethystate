//! Everything a declaration becomes when it runs in a browser, against a
//! store on the other side of a Tauri command.
//!
//! Nothing here reaches the store: a value arrives with the initial scan and
//! goes back through a command, so what is generated is the same struct with
//! a different thing behind every field.

use crate::amethystate::model::{Field, Placement, Schema, Shape};
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

pub(crate) fn generate(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let (vis, name, attrs) = (&schema.vis, &schema.name, &schema.forwarded);
    let fields = &schema.fields;
    let is_root = schema.is_root();
    let prefix_str = schema
        .prefix
        .as_ref()
        .map(Placement::path)
        .unwrap_or_default();

    let backend_ty = quote! { ::amethystate::tauri::TauriBackend };

    let held = |field: &Field| {
        let ty = &field.ty;
        match &field.shape {
            Shape::Node { .. } => {
                let nested_type = get_type_ident(ty);
                quote! { #nested_type }
            }
            Shape::Map { key, value, .. } => {
                quote! { #crate_name::client::ReactiveMap<#key, #value, #backend_ty> }
            }
            Shape::Leaf { .. } | Shape::Volatile { .. } => {
                quote! { #crate_name::client::Field<#ty, #backend_ty> }
            }
        }
    };

    let struct_fields = fields.iter().map(|field| {
        let fname = &field.ident;
        let fvis = &field.vis;
        let ty = held(field);
        let carried = &field.forwarded;

        quote! { #(#carried)* #fvis #fname: #ty }
    });

    let methods = fields.iter().map(|field| {
        let fname = &field.ident;
        let ty = held(field);
        let carried = &field.forwarded;

        quote! { #(#carried)* pub fn #fname(&self) -> #ty { self.#fname.clone() } }
    });

    let init_fields = fields.iter().map(|field| {
        let fname = &field.ident;
        let key_suffix = &field.stored.value;

        let full_key = if prefix_str == "." {
            key_suffix.clone()
        } else {
            format!("{prefix_str}.{key_suffix}")
        };
        let ty = &field.ty;

        match &field.shape {
            Shape::Node { .. } => {
                let nested_type = get_type_ident(ty);
                quote! { #fname: #nested_type::new_with_id(#full_key, &initial, store, instance_id) }
            }
            Shape::Map { key, value, .. } => quote! {
                #fname: {
                    let mut map_init = ::std::collections::HashMap::new();
                    let map_prefix = format!("{}.", #full_key);
                    for (k, v) in initial {
                        if let Some(sub_key) = k.strip_prefix(&map_prefix) {
                            if let Ok(parsed_k) = <#key as ::std::str::FromStr>::from_str(sub_key) {
                                if let Ok(parsed_v) = store.decode::<#value>(v) {
                                    map_init.insert(parsed_k, parsed_v);
                                }
                            }
                        }
                    }
                    #crate_name::client::ReactiveMap::new_with_backend_and_id(#full_key, map_init, store.clone(), instance_id)
                }
            },
            Shape::Leaf { default, .. } | Shape::Volatile { default } => quote! {
                #fname: {
                    let val = initial.get(#full_key)
                        .and_then(|v| store.decode::<#ty>(v).ok())
                        .unwrap_or_else(|| #default);
                    #crate_name::client::Field::new_with_backend_and_id(#full_key, val, store.clone(), instance_id)
                }
            },
        }
    });

    let load_impl = if is_root {
        quote! {
            impl #crate_name::client::AmeStateSliceAsync<#backend_ty> for #name {
                type Error = <#backend_ty as #crate_name::client::AmeBackendAsync>::Error;

                async fn load_async(store: &#backend_ty) -> ::std::result::Result<Self, Self::Error> {
                    use #crate_name::client::AmeBackendAsync;
                    let scan_prefix = if #prefix_str == "." { "" } else { #prefix_str };
                    let raw_entries = store.scan_prefix(scan_prefix).await?;
                    let mut initial = ::std::collections::HashMap::new();
                    for (k, v) in raw_entries {
                        initial.insert(k, v);
                    }

                    Ok(Self::new_with_id(&initial, store, #crate_name::uuid::Uuid::new_v4()))
                }
            }

            impl #name {
                pub fn new_with_id(initial: &::std::collections::HashMap<String, <#backend_ty as #crate_name::client::AmeBackendAsync>::Raw>, store: &#backend_ty, instance_id: #crate_name::uuid::Uuid) -> Self {
                    use #crate_name::client::AmeBackendAsync;
                    Self {
                        __amethystate_instance_id: instance_id,
                        #(#init_fields,)*
                    }
                }
            }
        }
    } else {
        let nested_init_fields = fields.iter().map(|field| {
            let fname = &field.ident;
            let key_str = &field.stored.value;
            let ty = &field.ty;

            match &field.shape {
                Shape::Node { .. } => {
                    let nested_type = get_type_ident(ty);
                    quote! { #fname: #nested_type::new_with_id(&format!("{}.{}", prefix, #key_str), initial, store, instance_id) }
                }
                Shape::Map { key, value, .. } => quote! {
                    #fname: {
                        let mut map_init = ::std::collections::HashMap::new();
                        let map_prefix = if prefix == "." { format!("{}.", #key_str) } else { format!("{}.{}.", prefix, #key_str) };
                        for (k, v) in initial {
                            if let Some(sub_key) = k.strip_prefix(&map_prefix) {
                                if let Ok(parsed_k) = <#key as ::std::str::FromStr>::from_str(sub_key) {
                                    if let Ok(parsed_v) = store.decode::<#value>(v) {
                                        map_init.insert(parsed_k, parsed_v);
                                    }
                                }
                            }
                        }
                        let map_key = if prefix == "." { #key_str.to_string() } else { format!("{}.{}", prefix, #key_str) };
                        #crate_name::client::ReactiveMap::new_with_backend_and_id(map_key, map_init, store.clone(), instance_id)
                    }
                },
                Shape::Leaf { default, .. } | Shape::Volatile { default } => quote! {
                    #fname: {
                        let full_key = if prefix == "." { #key_str.to_string() } else { format!("{}.{}", prefix, #key_str) };
                        let val = initial.get(&full_key)
                            .and_then(|v| store.decode::<#ty>(v).ok())
                            .unwrap_or_else(|| #default);
                        #crate_name::client::Field::new_with_backend_and_id(full_key, val, store.clone(), instance_id)
                    }
                },
            }
        });

        quote! {
            impl #name {
                pub fn new(prefix: &str, initial: &::std::collections::HashMap<String, <#backend_ty as #crate_name::client::AmeBackendAsync>::Raw>, store: &#backend_ty) -> Self {
                    Self::new_with_id(prefix, initial, store, #crate_name::uuid::Uuid::new_v4())
                }

                pub fn new_with_id(prefix: &str, initial: &::std::collections::HashMap<String, <#backend_ty as #crate_name::client::AmeBackendAsync>::Raw>, store: &#backend_ty, instance_id: #crate_name::uuid::Uuid) -> Self {
                    use #crate_name::client::AmeBackendAsync;
                    Self {
                        __amethystate_instance_id: instance_id,
                        #(#nested_init_fields,)*
                    }
                }
            }
        }
    };

    let fork_fields = fields.iter().map(|field| {
        let fname = &field.ident;
        quote! { #fname: self.#fname.fork_with_id(new_id) }
    });

    quote! {
        #[derive(Clone, Debug, Eq, PartialEq)]
        #(#attrs)* #vis struct #name {
            __amethystate_instance_id: #crate_name::uuid::Uuid,
            #(#struct_fields,)*
        }

        #load_impl

        impl #name {
            #(#methods)*

            pub fn fork(&self) -> Self {
                self.fork_with_id(#crate_name::uuid::Uuid::new_v4())
            }

            #[doc(hidden)]
            pub fn fork_with_id(&self, new_id: #crate_name::uuid::Uuid) -> Self {
                Self {
                    __amethystate_instance_id: new_id,
                    #(#fork_fields,)*
                }
            }
        }
    }
}

fn get_type_ident(ty: &syn::Type) -> proc_macro2::TokenStream {
    if let syn::Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        let ident = &segment.ident;
        return quote! { #ident };
    }
    quote! { #ty }
}
