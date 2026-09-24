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
    let prefix = super::path_literal(
        crate_name,
        &schema
            .prefix
            .as_ref()
            .map(Placement::path)
            .unwrap_or_default(),
    );

    let backend_ty = quote! { ::amethystate::tauri::TauriBackend };
    let path_ty = quote! { #crate_name::store::StorePath };
    let raw_ty = quote! { <#backend_ty as #crate_name::client::AmeBackendAsync>::Raw };

    let held = |field: &Field| {
        let ty = &field.ty;
        match &field.shape {
            Shape::Node { .. } => {
                let nested_type = get_type_ident(ty);
                quote! { #nested_type }
            }
            Shape::Stored { .. } => match crate::amethystate::model::written_map(ty) {
                Some((key, value)) => {
                    quote! { #crate_name::client::ReactiveMap<#key, #value, #backend_ty> }
                }
                None => quote! { #crate_name::client::Field<#ty, #backend_ty> },
            },
            Shape::Volatile { .. } => {
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

    let init_fields: Vec<TokenStream2> = fields
        .iter()
        .map(|field| {
            let fname = &field.ident;
            let key = super::path_literal(crate_name, &field.stored.value);
            let ty = &field.ty;

            let read = |seed: TokenStream2| {
                quote! {
                    #fname: {
                        let path = at.join(&#key);
                        let val = initial
                            .get(&path)
                            .and_then(|raw| store.decode::<#ty>(raw).ok())
                            .unwrap_or_else(|| #seed);
                        #crate_name::client::Field::new_with_backend_and_id(path, val, store.clone(), instance_id)
                    }
                }
            };

            match &field.shape {
                Shape::Node { .. } => {
                    let nested_type = get_type_ident(ty);
                    quote! { #fname: #nested_type::new_with_id(&at.join(&#key), initial, store, instance_id) }
                }
                Shape::Stored { default, .. } => match crate::amethystate::model::written_map(ty) {
                    Some((key_ty, value_ty)) => quote! {
                        #fname: {
                            let path = at.join(&#key);
                            let mut entries = ::std::collections::HashMap::new();
                            for (stored, raw) in initial {
                                if stored.len() != path.len() + 1 {
                                    continue;
                                }
                                let Some(name) = path.entry_name(stored) else {
                                    continue;
                                };
                                if let (Ok(k), Ok(v)) = (
                                    <#key_ty as ::std::str::FromStr>::from_str(name.as_str()),
                                    store.decode::<#value_ty>(raw),
                                ) {
                                    entries.insert(k, v);
                                }
                            }
                            #crate_name::client::ReactiveMap::new_with_backend_and_id(path, entries, store.clone(), instance_id)
                        }
                    },
                    None => read(super::seed_tokens(default)),
                },
                Shape::Volatile { default } => read(quote! { #default }),
            }
        })
        .collect();

    let load_impl = if is_root {
        quote! {
            impl #crate_name::client::AmeStateSliceAsync<#backend_ty> for #name {
                type Error = #crate_name::errors::Report<<#backend_ty as #crate_name::client::AmeBackendAsync>::Error>;

                async fn load_async(store: &#backend_ty) -> ::std::result::Result<Self, Self::Error> {
                    use #crate_name::client::AmeBackendAsync;
                    let initial: ::std::collections::HashMap<#path_ty, #raw_ty> =
                        store.scan_prefix(&#prefix).await?.into_iter().collect();

                    Ok(Self::new_with_id(&initial, store, #crate_name::uuid::Uuid::new_v4()))
                }
            }

            impl #name {
                pub fn new_with_id(
                    initial: &::std::collections::HashMap<#path_ty, #raw_ty>,
                    store: &#backend_ty,
                    instance_id: #crate_name::uuid::Uuid,
                ) -> Self {
                    use #crate_name::client::AmeBackendAsync;
                    let at: #path_ty = #prefix;
                    Self {
                        __amethystate_instance_id: instance_id,
                        #(#init_fields,)*
                    }
                }
            }
        }
    } else {
        quote! {
            impl #name {
                pub fn new(
                    at: &#path_ty,
                    initial: &::std::collections::HashMap<#path_ty, #raw_ty>,
                    store: &#backend_ty,
                ) -> Self {
                    Self::new_with_id(at, initial, store, #crate_name::uuid::Uuid::new_v4())
                }

                pub fn new_with_id(
                    at: &#path_ty,
                    initial: &::std::collections::HashMap<#path_ty, #raw_ty>,
                    store: &#backend_ty,
                    instance_id: #crate_name::uuid::Uuid,
                ) -> Self {
                    use #crate_name::client::AmeBackendAsync;
                    Self {
                        __amethystate_instance_id: instance_id,
                        #(#init_fields,)*
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
