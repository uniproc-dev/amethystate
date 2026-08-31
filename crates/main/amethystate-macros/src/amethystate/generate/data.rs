use super::RpMode;
use crate::amethystate::generate::{parse_default, path_literal};
use amethystate_macros_core::{MacroArgs, StoreFieldEntry};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::Ident;

pub(crate) fn persistent_fields(entries: &[StoreFieldEntry]) -> Vec<&StoreFieldEntry> {
    entries.iter().filter(|e| !e.volatile).collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn data_impl(
    crate_name: &TokenStream2,
    vis: &syn::Visibility,
    name: &Ident,
    attrs: &[syn::Attribute],
    prefix: Option<String>,
    entries: &[StoreFieldEntry],
    macro_args: &MacroArgs,
    rp_mode: RpMode,
) -> TokenStream2 {
    let forwarded_derives: Vec<&syn::Attribute> = attrs
        .iter()
        .filter(|a| a.path().is_ident("derive"))
        .collect();

    let mut p_fields = persistent_fields(entries);

    p_fields.sort_by(|a, b| {
        a.ident
            .as_ref()
            .unwrap()
            .to_string()
            .cmp(&b.ident.as_ref().unwrap().to_string())
    });

    let data_struct_name = format_ident!("{}_Data", name);

    let data_fields = p_fields.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let ty = &e.ty;
        if e.nested {
            quote! { pub #fname: <#ty as #crate_name::AmeState>::Data }
        } else if let Some((k, v)) = e.get_map_types() {
            quote! { pub #fname: ::std::collections::HashMap<#k, #v> }
        } else {
            quote! { pub #fname: #ty }
        }
    });

    let version_val = macro_args.version.unwrap_or(0);

    let field_descriptors = p_fields.iter().map(|e| {
        let fname_str = e.stored_name();
        let ty = &e.ty;
        let type_name = quote!(#ty).to_string().replace(" ", "");

        if e.nested {
            quote! {
                #crate_name::migration::fields::FieldDescriptor {
                    name: #fname_str,
                    type_hash: 0xDEADBEEF ^ < <#ty as #crate_name::AmeState>::Data as #crate_name::migration::types::AmeType>::TYPE_HASH,
                    type_name: #type_name,
                    role: #crate_name::migration::fields::Role::Node,
                    optional: false,
                    children: < <#ty as #crate_name::AmeState>::Data as #crate_name::migration::fields::AmeStateFields>::FIELDS,
                }
            }
        } else if let Some((k, v)) = e.get_map_types() {
            quote! {
                #crate_name::migration::fields::FieldDescriptor {
                    name: #fname_str,
                    type_hash: <::std::collections::HashMap<#k, #v> as #crate_name::migration::types::AmeType>::TYPE_HASH,
                    type_name: #type_name,
                    role: <#crate_name::shape::Probe<#ty>>::ROLE,
                    optional: <#crate_name::shape::Probe<#ty>>::OPTIONAL,
                    children: &[],
                }
            }
        } else {
            quote! {
                #crate_name::migration::fields::FieldDescriptor {
                    name: #fname_str,
                    type_hash: <#ty as #crate_name::migration::types::AmeType>::TYPE_HASH,
                    type_name: #type_name,
                    role: <#crate_name::shape::Probe<#ty>>::ROLE,
                    optional: <#crate_name::shape::Probe<#ty>>::OPTIONAL,
                    children: &[],
                }
            }
        }
    });

    let shape_checks = p_fields.iter().filter(|e| !e.nested).map(|e| {
        let fname_str = e.ident.as_ref().unwrap().to_string();
        let ty = &e.ty;

        let (expected, message) = if e.get_map_types().is_some() {
            (
                quote! { #crate_name::migration::fields::Role::Map },
                format!(
                    "field `{fname_str}` is spelled as a ReactiveMap but is not one - \
                     the name belongs to another type here"
                ),
            )
        } else {
            (
                quote! { #crate_name::migration::fields::Role::Field },
                format!(
                    "field `{fname_str}` is a ReactiveMap, and was taken for a plain value \
                     because the type is not written as one - spell it `ReactiveMap<K, V>` \
                     at the field rather than through an alias"
                ),
            )
        };

        quote! {
            const _: () = assert!(
                <#crate_name::shape::Probe<#ty>>::ROLE.same(#expected),
                #message
            );
        }
    });

    let load_fields = p_fields.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let key = e.stored_name();
        let ty = &e.ty;

        if e.nested {
            quote! {
                #fname: {
                    let mut sub_ctx = ctx.scoped(#key);
                    < <#ty as #crate_name::AmeState>::Data as #crate_name::migration::fields::AmeStateFields>::load_struct(&mut sub_ctx)?
                }
            }
        } else if let Some((k, v)) = e.get_map_types() {
            quote! {
                #fname: ctx.scan_map::<#k, #v>(#key)?
            }
        } else {
            let fallback = e
                .default
                .as_ref()
                .map(parse_default)
                .unwrap_or_else(|| quote! { <#ty as ::std::default::Default>::default() });
            quote! {
                #fname: ctx.get::<#ty>(#key)?.unwrap_or_else(|| #fallback)
            }
        }
    });

    let save_fields = p_fields.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let key = e.stored_name();

        if e.nested {
            quote! {
                {
                    let mut sub_ctx = ctx.scoped(#key);
                    self.#fname.save_struct(&mut sub_ctx)?;
                }
            }
        } else if e.get_map_types().is_some() {
            quote! {
                for (k, v) in &self.#fname {
                    let full_key = format!("{}.{}", #key, k);
                    ctx.set(&full_key, v)?;
                }
            }
        } else {
            quote! { ctx.set(#key, &self.#fname)?; }
        }
    });

    let struct_policy = super::read_policy(macro_args.on_unreadable.as_ref())
        .ok()
        .flatten();

    let store_load_fields = p_fields.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let key = e.stored_name();
        let key_path = path_literal(crate_name, &key);
        let ty = &e.ty;
        if e.nested {
            let data_ty = get_data_type(ty);
            quote! {
                #fname: <#data_ty>::__amethystate_load_from(store, &prefix.join(&#key_path))?
            }
        } else if let Some((k, v)) = e.get_map_types() {
            quote! {
                #fname: #crate_name::store::load_map::<#k, #v>(store, &prefix.join(&#key_path))?
            }
        } else {
            let fallback = e
                .default
                .as_ref()
                .map(parse_default)
                .unwrap_or_else(|| quote! { <#ty as ::std::default::Default>::default() });

            let Some(check) = &e.check else {
                return quote! {
                    #fname: <#crate_name::Store as #crate_name::StoreExt>::get::<#ty>(store, &prefix.join(&#key_path))?.unwrap_or_else(|| #fallback)
                };
            };

            let policy = match super::read_policy(e.on_unreadable.as_ref())
                .ok()
                .flatten()
                .or_else(|| struct_policy.clone())
                .as_deref()
            {
                Some("UseDefault") => quote!(#crate_name::store::OnUnreadable::UseDefault),
                _ => quote!(#crate_name::store::OnUnreadable::Refuse),
            };

            quote! {
                #fname: {
                    let __ame_path = prefix.join(&#key_path);
                    match <#crate_name::Store as #crate_name::StoreExt>::get::<#ty>(store, &__ame_path)? {
                        ::core::option::Option::Some(__ame_value) => match #check(&__ame_value, store.context()) {
                            ::core::result::Result::Ok(()) => __ame_value,
                            ::core::result::Result::Err(__ame_invalid) => #crate_name::store::refused_or_default(
                                &__ame_path,
                                __ame_invalid,
                                #policy,
                                #fallback,
                            )?,
                        },
                        ::core::option::Option::None => #fallback,
                    }
                }
            }
        }
    });

    let store_save_fields = p_fields.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let key = e.stored_name();
        let key_path = path_literal(crate_name, &key);

        if e.nested {
            quote! {
                self.#fname.__amethystate_save_to(store, &prefix.join(&#key_path))?;
            }
        } else if e.get_map_types().is_some() {
            quote! {
                {
                    let path = prefix.join(&#key_path);
                    for (k, v) in &self.#fname {
                        let full_path = #crate_name::store::entry_path(&path, k.to_string())?;
                        <#crate_name::Store as #crate_name::StoreExt>::set(store, &full_path, v)?;
                    }
                }
            }
        } else {
            quote! {
                <#crate_name::Store as #crate_name::StoreExt>::set(&store, &prefix.join(&#key_path), &self.#fname)?;
            }
        }
    });

    let fields_for_hash = p_fields
        .iter()
        .map(|e| {
            let fname_str = e.ident.as_ref().unwrap().to_string();
            let ty = &e.ty;
            let field_ty = if e.nested {
                quote! { <#ty as #crate_name::AmeState>::Data }
            } else if let Some((k, v)) = e.get_map_types() {
                quote! { ::std::collections::HashMap<#k, #v> }
            } else {
                quote! { #ty }
            };
            (fname_str, field_ty)
        })
        .collect::<Vec<_>>();

    let recursive_hash_expr = crate::hash::gen_recursive_type_hash(crate_name, fields_for_hash);

    let prefix_expr = prefix.clone().unwrap_or_default();
    let prefix_path = path_literal(crate_name, &prefix_expr);
    let is_root = prefix.is_some();

    let persistent_wrapper_tokens = match rp_mode {
        RpMode::Reactive => quote! {},
        RpMode::Persistent => {
            quote! {
                #[derive(Clone)]
                #(#attrs)* #vis struct #name {
                    inner: #data_struct_name,
                    store: #crate_name::Store,
                    prefix: #crate_name::store::StorePath,
                }

                impl ::std::ops::Deref for #name {
                    type Target = #data_struct_name;

                    fn deref(&self) -> &Self::Target {
                        &self.inner
                    }
                }

                impl ::std::ops::DerefMut for #name {
                    fn deref_mut(&mut self) -> &mut Self::Target {
                        &mut self.inner
                    }
                }

                impl #name {
                    pub fn save_lazy(&self) -> #crate_name::StorageResult<()> {
                        self.inner
                            .__amethystate_save_to(&self.store, &self.prefix)
                    }

                    pub fn mutate_lazy(&mut self, f: impl FnOnce(&mut #data_struct_name)) -> #crate_name::StorageResult<()> {
                        f(&mut self.inner);
                        self.save_lazy()
                    }

                    pub fn mutate(&mut self, f: impl FnOnce(&mut #data_struct_name)) -> #crate_name::StorageResult<()> {
                        f(&mut self.inner);
                        self.save()
                    }

                    pub fn save(&self) -> #crate_name::StorageResult<()> {
                        self.save_lazy()?;
                        <#crate_name::Store as #crate_name::StoreBackend>::flush_prefix(&self.store, &self.prefix)
                    }

                    pub fn load_with(store: &#crate_name::Store) -> #crate_name::StorageResult<Self> {
                        Ok(Self {
                            inner: #data_struct_name::__amethystate_load_from(store, &#prefix_path)?,
                            store: store.clone(),
                            prefix: #prefix_path,
                        })
                    }
                }

                impl #name {
                    pub fn load() -> #crate_name::StorageResult<Self> {
                        let store = #crate_name::global_store();
                        Self::load_with(&store)
                    }
                }
            }
        }
        RpMode::Both => {
            let persisted_struct_name = format_ident!("{}_Persistent", name);
            quote! {
                #[allow(non_camel_case_types)]
                #[derive(Clone)]
                #(#forwarded_derives)*
                pub struct #persisted_struct_name {
                    inner: #data_struct_name,
                    store: #crate_name::Store,
                    prefix: #crate_name::store::StorePath,
                }

                impl ::std::ops::Deref for #persisted_struct_name {
                    type Target = #data_struct_name;

                    fn deref(&self) -> &Self::Target {
                        &self.inner
                    }
                }

                impl ::std::ops::DerefMut for #persisted_struct_name {
                    fn deref_mut(&mut self) -> &mut Self::Target {
                        &mut self.inner
                    }
                }

                impl #persisted_struct_name {
                    pub fn save_lazy(&self) -> #crate_name::StorageResult<()> {
                        self.inner
                            .__amethystate_save_to(&self.store, &self.prefix)
                    }

                    pub fn mutate_lazy(&mut self, f: impl FnOnce(&mut #data_struct_name)) -> #crate_name::StorageResult<()> {
                        f(&mut self.inner);
                        self.save_lazy()
                    }

                    pub fn mutate(&mut self, f: impl FnOnce(&mut #data_struct_name)) -> #crate_name::StorageResult<()> {
                        f(&mut self.inner);
                        self.save()
                    }

                    pub fn save(&self) -> #crate_name::StorageResult<()> {
                        self.save_lazy()?;
                        <#crate_name::Store as #crate_name::StoreBackend>::flush_prefix(&self.store, &self.prefix)
                    }
                }

                impl #name {
                    pub fn load_with(store: &#crate_name::Store) -> #crate_name::StorageResult<#persisted_struct_name> {
                        Ok(#persisted_struct_name {
                            inner: #data_struct_name::__amethystate_load_from(store, &#prefix_path)?,
                            store: store.clone(),
                            prefix: #prefix_path,
                        })
                    }
                }

                impl #name {
                    pub fn load() -> #crate_name::StorageResult<#persisted_struct_name> {
                        let store = #crate_name::global_store();
                        Self::load_with(&store)
                    }
                }
            }
        }
    };

    let gen_load_save_helpers = !(is_root && matches!(rp_mode, RpMode::Reactive));

    let load_save_helpers = if gen_load_save_helpers {
        quote! {
            #[doc(hidden)]
            pub fn __amethystate_load_from(
                store: &#crate_name::Store,
                prefix: &#crate_name::store::StorePath,
            ) -> #crate_name::StorageResult<Self> {
                Ok(Self {
                    #(#store_load_fields,)*
                })
            }

            #[doc(hidden)]
            pub fn __amethystate_save_to(
                &self,
                store: &#crate_name::Store,
                prefix: &#crate_name::store::StorePath,
            ) -> #crate_name::StorageResult<()> {
                #(#store_save_fields)*
                Ok(())
            }
        }
    } else {
        quote! {}
    };

    quote! {
        #[derive(#crate_name::serde::Serialize, #crate_name::serde::Deserialize, Default, Clone)]
        #(#forwarded_derives)*
        #[serde(crate = "::amethystate::serde")]
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #data_struct_name {
            #(#data_fields,)*
        }

        #persistent_wrapper_tokens

        impl #data_struct_name {
            #load_save_helpers
        }

        impl #crate_name::migration::types::AmeType for #data_struct_name {
            const TYPE_HASH: u32 = #recursive_hash_expr;
            const TYPE_NAME: &'static str = stringify!(#data_struct_name);
        }

       impl #crate_name::migration::fields::AmeStateFields for #data_struct_name {
            const FIELDS: &'static [#crate_name::migration::fields::FieldDescriptor] = {
                #[allow(unused_imports)]
                use #crate_name::shape::AnyShape as _;

                #(#shape_checks)*

                &[
                    #(#field_descriptors),*
                ]
            };
            const VERSION: u32 = #version_val;
            const SCHEMA_HASH: u32 = #crate_name::migration::types::schema_hash(Self::FIELDS);
            const PARENT_PREFIX: &'static str = #prefix_expr;
            const MIGRATION_DEPS: &'static [&'static str] = &[];

            fn load_struct(ctx: &mut #crate_name::MigrationContext) -> #crate_name::StorageResult<Self> {
                Ok(Self {
                    #(#load_fields,)*
                })
            }

            fn save_struct(&self, ctx: &mut #crate_name::MigrationContext) -> #crate_name::StorageResult<()> {
                #(#save_fields)*
                Ok(())
            }
        }

        impl #crate_name::AmeState for #name {
            type Data = #data_struct_name;
        }
    }
}

fn get_data_type(ty: &syn::Type) -> proc_macro2::TokenStream {
    if let syn::Type::Path(type_path) = ty {
        let mut path = type_path.path.clone();
        if let Some(last) = path.segments.last_mut() {
            last.arguments = syn::PathArguments::None;
            last.ident = quote::format_ident!("{}_Data", last.ident);
        }
        quote::quote! { #path }
    } else {
        quote::quote! { #ty }
    }
}
