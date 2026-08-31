use crate::amethystate::MacroArgs;
use crate::amethystate::generate::parse_default;
use amethystate_macros_core::StoreFieldEntry;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, Visibility};

pub fn generate_wasm_code(
    crate_name: TokenStream2,
    vis: &Visibility,
    name: &Ident,
    attrs: &[syn::Attribute],
    prefix: Option<String>,
    entries: &[StoreFieldEntry],
    _macro_args: MacroArgs,
) -> TokenStream2 {
    let is_root = prefix.is_some();
    let prefix_str = prefix.unwrap_or_default();

    let backend_ty = quote! { ::amethystate::tauri::TauriBackend };

    let struct_fields = entries.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let fvis = &e.vis;
        let ty = &e.ty;

        if e.nested {
            let nested_type = get_type_ident(ty);
            quote! { #fvis #fname: #nested_type }
        } else if let Some((k, v)) = e.get_map_types() {
            quote! { #fvis #fname: #crate_name::client::ReactiveMap<#k, #v, #backend_ty> }
        } else {
            quote! { #fvis #fname: #crate_name::client::Field<#ty, #backend_ty> }
        }
    });

    let methods = entries.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let ty = &e.ty;

        if e.nested {
            let nested_type = get_type_ident(ty);
            quote! { pub fn #fname(&self) -> #nested_type { self.#fname.clone() } }
        } else if let Some((k, v)) = e.get_map_types() {
            quote! { pub fn #fname(&self) -> #crate_name::client::ReactiveMap<#k, #v, #backend_ty> { self.#fname.clone() } }
        } else {
            quote! { pub fn #fname(&self) -> #crate_name::client::Field<#ty, #backend_ty> { self.#fname.clone() } }
        }
    });

    let init_fields = entries.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let key_suffix = e.stored_name();

        let full_key = if prefix_str == "." {
            key_suffix
        } else {
            format!("{prefix_str}.{key_suffix}")
        };


        let ty = &e.ty;
        let fallback = e.default.as_ref().map(parse_default).unwrap_or_else(|| quote! { ::std::default::Default::default() });

        if e.nested {
            let nested_type = get_type_ident(ty);
            quote! { #fname: #nested_type::new_with_id(#full_key, &initial, store, instance_id) }
        } else if let Some((k, v)) = e.get_map_types() {
            quote! {
                #fname: {
                    let mut map_init = ::std::collections::HashMap::new();
                    let map_prefix = format!("{}.", #full_key);
                    for (k, v) in initial {
                        if let Some(sub_key) = k.strip_prefix(&map_prefix) {
                            if let Ok(parsed_k) = <#k as ::std::str::FromStr>::from_str(sub_key) {
                                if let Ok(parsed_v) = store.decode::<#v>(v) {
                                    map_init.insert(parsed_k, parsed_v);
                                }
                            }
                        }
                    }
                    #crate_name::client::ReactiveMap::new_with_backend_and_id(#full_key, map_init, store.clone(), instance_id)
                }
            }
        } else {
            quote! {
                #fname: {
                    let val = initial.get(#full_key)
                        .and_then(|v| store.decode::<#ty>(v).ok())
                        .unwrap_or_else(|| #fallback);
                    #crate_name::client::Field::new_with_backend_and_id(#full_key, val, store.clone(), instance_id)
                }
            }
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
        let nested_init_fields = entries.iter().map(|e| {
            let fname = e.ident.as_ref().unwrap();
            let key_str = e.stored_name();
            let ty = &e.ty;
            let fallback = e.default.as_ref().map(parse_default).unwrap_or_else(|| quote! { ::std::default::Default::default() });

            if e.nested {
                let nested_type = get_type_ident(ty);
                quote! { #fname: #nested_type::new_with_id(&format!("{}.{}", prefix, #key_str), initial, store, instance_id) }
            } else if let Some((k, v)) = e.get_map_types() {
                quote! {
                    #fname: {
                        let mut map_init = ::std::collections::HashMap::new();
                        let map_prefix = if prefix == "." { format!("{}.", #key_str) } else { format!("{}.{}.", prefix, #key_str) };
                        for (k, v) in initial {
                            if let Some(sub_key) = k.strip_prefix(&map_prefix) {
                                if let Ok(parsed_k) = <#k as ::std::str::FromStr>::from_str(sub_key) {
                                    if let Ok(parsed_v) = store.decode::<#v>(v) {
                                        map_init.insert(parsed_k, parsed_v);
                                    }
                                }
                            }
                        }
                        let map_key = if prefix == "." { #key_str.to_string() } else { format!("{}.{}", prefix, #key_str) };
                        #crate_name::client::ReactiveMap::new_with_backend_and_id(map_key, map_init, store.clone(), instance_id)
                    }
                }
            } else {
                quote! {
                    #fname: {
                        let full_key = if prefix == "." { #key_str.to_string() } else { format!("{}.{}", prefix, #key_str) };
                        let val = initial.get(&full_key)
                            .and_then(|v| store.decode::<#ty>(v).ok())
                            .unwrap_or_else(|| #fallback);
                        #crate_name::client::Field::new_with_backend_and_id(full_key, val, store.clone(), instance_id)
                    }
                }
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

    let fork_fields = entries.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
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
