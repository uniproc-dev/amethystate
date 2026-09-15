use crate::amethystate::generate::{levels_literal, path_literal, static_path_literal};
use crate::amethystate::model::{Field, Mode, OnUnreadable, Placement, Schema, Shape};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;

/// How this field is stored, or the type's own form where it said nothing.
fn stored_as_tokens(crate_name: &TokenStream2, field: &Field) -> TokenStream2 {
    match &field.shape {
        Shape::Stored {
            stored_as: Some(how),
            ..
        } => super::init::stored_as(crate_name, &field.ty, how),
        _ => quote! { #crate_name::store::StoredAs::default() },
    }
}

pub(crate) fn data_impl(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let (vis, name, attrs) = (&schema.vis, &schema.name, &schema.forwarded);
    let serde_path = format!("{}::serde", quote!(#crate_name));
    let prefix = schema.prefix.as_ref().map(Placement::path);
    let mode = schema.mode;

    let forwarded_derives = super::derives_without(attrs, &["Clone", "Serialize", "Deserialize"]);

    let mut p_fields: Vec<&Field> = schema.stored().collect();

    p_fields.sort_by_key(|field| field.ident.to_string());

    let data_struct_name = format_ident!("{}_Data", name);

    let data_fields = p_fields.iter().map(|field| {
        let fname = &field.ident;
        let ty = &field.ty;
        let held = match &field.shape {
            Shape::Node { .. } => quote! { <#ty as #crate_name::AmeState>::Data },
            _ => quote! { <#ty as #crate_name::shape::Kind>::Data },
        };
        let carried = &field.forwarded;

        quote! { #(#carried)* pub #fname: #held }
    });

    let version_val = schema.version;
    let id_val = match &schema.id {
        Some(written) => {
            let written = written.value.as_str();
            quote! { Some(#written) }
        }
        None => quote! { None },
    };

    let field_descriptors = p_fields.iter().map(|field| {
        let fname_str = static_path_literal(crate_name, &field.stored.value);
        let declared = field.ident.to_string();
        let ty = &field.ty;
        let type_name = quote!(#ty).to_string().replace(" ", "");

        match &field.shape {
            Shape::Node { flattened } => quote! {
                #crate_name::migration::fields::FieldDescriptor {
                    name: #fname_str,
                    declared: #declared,
                    type_name: #type_name,
                    role: #crate_name::migration::fields::Role::Node,
                    optional: false,
                    children: < <#ty as #crate_name::AmeState>::Data as #crate_name::migration::fields::AmeStateFields>::FIELDS,
                    flattened: #flattened,
                }
            },
            _ => quote! {
                #crate_name::migration::fields::FieldDescriptor {
                    name: #fname_str,
                    declared: #declared,
                    type_name: #type_name,
                    role: <#crate_name::shape::Probe<#ty>>::ROLE,
                    optional: <#crate_name::shape::Probe<#ty>>::OPTIONAL,
                    children: &[],
                    flattened: false,
                }
            },
        }
    });

    let flat: Vec<&&Field> = p_fields
        .iter()
        .filter(|field| matches!(field.shape, Shape::Node { flattened: true }))
        .collect();

    let own_names: Vec<String> = p_fields
        .iter()
        .filter(|field| !matches!(field.shape, Shape::Node { flattened: true }))
        .map(|field| field.stored.value.clone())
        .collect();

    let mut flatten_checks: Vec<TokenStream2> = Vec::new();

    for (at, one) in flat.iter().enumerate() {
        let held = &one.ident;
        let ty = &one.ty;

        if !own_names.is_empty() {
            let message = format!(
                "`{held}` is flattened into `{name}`, so its own fields are stored at this level - and one of them is spelled the same as a field written here. Two paths cannot be the same path: rename one, or drop the flatten and let `{held}` keep its segment"
            );
            let names = own_names.iter().map(|n| quote!(#n));

            flatten_checks.push(quote! {
                const _: () = assert!(
                    !#crate_name::migration::fields::brings_any(
                        < <#ty as #crate_name::AmeState>::Data as #crate_name::migration::fields::AmeStateFields>::FIELDS,
                        &[#(#names),*],
                    ),
                    #message
                );
            });
        }

        for other in &flat[at + 1..] {
            let beside = &other.ident;
            let other_ty = &other.ty;
            let message = format!(
                "`{held}` and `{beside}` are both flattened into `{name}`, and they have a field name in common. Flattened, each stores its fields at this level, so the two would write over each other"
            );

            flatten_checks.push(quote! {
                const _: () = assert!(
                    !#crate_name::migration::fields::overlap(
                        < <#ty as #crate_name::AmeState>::Data as #crate_name::migration::fields::AmeStateFields>::FIELDS,
                        < <#other_ty as #crate_name::AmeState>::Data as #crate_name::migration::fields::AmeStateFields>::FIELDS,
                    ),
                    #message
                );
            });
        }
    }

    let load_fields = p_fields.iter().map(|field| {
        let fname = &field.ident;
        let at = levels_literal(&field.stored.value);
        let ty = &field.ty;

        match &field.shape {
            Shape::Node { flattened } => {
                let sub_ctx = if *flattened {
                    quote!(ctx.here())
                } else {
                    quote!(ctx.scoped(#at))
                };
                quote! {
                    #fname: {
                        let mut sub_ctx = #sub_ctx;
                        < <#ty as #crate_name::AmeState>::Data as #crate_name::migration::fields::AmeStateFields>::load_struct(&mut sub_ctx)?
                    }
                }
            }
            Shape::Stored { default, .. } => {
                let stored_as = stored_as_tokens(crate_name, field);
                let seed = super::seed_tokens(default);
                quote! {
                    #fname: <#ty as #crate_name::shape::Kind>::load_step(
                        ctx,
                        #at,
                        #stored_as,
                        || #seed,
                    )?
                }
            }
            Shape::Volatile { .. } => unreachable!("a volatile field is never stored"),
        }
    });

    let save_fields = p_fields.iter().map(|field| {
        let fname = &field.ident;
        let at = levels_literal(&field.stored.value);
        let ty = &field.ty;

        match &field.shape {
            Shape::Node { flattened } => {
                let sub_ctx = if *flattened {
                    quote!(ctx.here())
                } else {
                    quote!(ctx.scoped(#at))
                };
                quote! {
                    {
                        let mut sub_ctx = #sub_ctx;
                        self.#fname.save_struct(&mut sub_ctx)?;
                    }
                }
            }
            Shape::Stored { .. } => {
                let stored_as = stored_as_tokens(crate_name, field);
                quote! {
                    <#ty as #crate_name::shape::Kind>::save_step(&self.#fname, ctx, #at, #stored_as)?;
                }
            }
            Shape::Volatile { .. } => unreachable!("a volatile field is never stored"),
        }
    });

    let struct_policy = schema.rules.on_unreadable.as_ref().map(|at| at.value);

    let store_load_fields = p_fields.iter().map(|field| {
        let fname = &field.ident;
        let key_path = path_literal(crate_name, &field.stored.value);
        let ty = &field.ty;

        let default = match &field.shape {
            Shape::Node { flattened } => {
                let data_ty = quote! { <#ty as #crate_name::AmeState>::Data };
                let under = if *flattened {
                    quote!(prefix.clone())
                } else {
                    quote!(prefix.join(&#key_path))
                };
                return quote! {
                    #fname: <#data_ty>::__amethystate_load_from(store, &#under)?
                };
            }
            Shape::Stored { default, .. } => default,
            Shape::Volatile { .. } => unreachable!("a volatile field is never stored"),
        };

        let seed = super::seed_tokens(default);

        let check = match field.rules.check.as_ref() {
            Some(check) => {
                let path = &check.value;
                quote_spanned! {check.span=> ::core::option::Option::Some(#path as #crate_name::store::Check<#ty>) }
            }
            None => quote!(::core::option::Option::None),
        };

        let stored_as = stored_as_tokens(crate_name, field);

        let policy = super::unreadable_tokens(
            crate_name,
            field
                .rules
                .on_unreadable
                .as_ref()
                .map(|at| at.value)
                .or(struct_policy)
                .unwrap_or(OnUnreadable::Refuse),
        );

        quote! {
            #fname: <#ty as #crate_name::shape::Kind>::load_plain(
                store,
                &prefix.join(&#key_path),
                #stored_as,
                #check,
                #policy,
                || #seed,
            )?
        }
    });

    let store_save_fields = p_fields.iter().map(|field| {
        let fname = &field.ident;
        let key_path = path_literal(crate_name, &field.stored.value);
        let stored_as = stored_as_tokens(crate_name, field);
        let ty = &field.ty;

        match &field.shape {
            Shape::Node { flattened } => {
                let under = if *flattened {
                    quote!(prefix.clone())
                } else {
                    quote!(prefix.join(&#key_path))
                };
                quote! {
                    self.#fname.__amethystate_save_to(store, &#under)?;
                }
            }
            Shape::Stored { .. } => quote! {
                <#ty as #crate_name::shape::Kind>::save_plain(
                    &self.#fname,
                    store,
                    &prefix.join(&#key_path),
                    #stored_as,
                )?;
            },
            Shape::Volatile { .. } => unreachable!("a volatile field is never stored"),
        }
    });

    let prefix_expr = prefix.clone().unwrap_or_default();
    let prefix_path = path_literal(crate_name, &prefix_expr);
    let prefix_static = static_path_literal(crate_name, &prefix_expr);
    let is_root = prefix.is_some();

    let persistent_wrapper_tokens = match mode {
        Mode::Reactive => quote! {},
        Mode::Persistent => {
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
                    pub fn save_lazy(&self) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                        self.inner
                            .__amethystate_save_to(&self.store, &self.prefix)
                    }

                    pub fn mutate_lazy(&mut self, f: impl FnOnce(&mut #data_struct_name)) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                        f(&mut self.inner);
                        self.save_lazy()
                    }

                    pub fn mutate(&mut self, f: impl FnOnce(&mut #data_struct_name)) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                        f(&mut self.inner);
                        self.save()
                    }

                    pub fn save(&self) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                        self.save_lazy()?;
                        #crate_name::Store::flush_prefix(&self.store, &self.prefix)
                            .map_err(|why| #crate_name::store::WriteValue::from_store(
                                &self.prefix,
                                ::core::convert::Into::into(why),
                            ))
                    }

                    pub fn load_with(store: &#crate_name::Store) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                        Ok(Self {
                            inner: #data_struct_name::__amethystate_load_from(store, &#prefix_path)?,
                            store: store.clone(),
                            prefix: #prefix_path,
                        })
                    }
                }

                impl #name {
                    pub fn load() -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                        let store = #crate_name::global_store();
                        Self::load_with(&store)
                    }
                }
            }
        }
        Mode::Both => {
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
                    pub fn save_lazy(&self) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                        self.inner
                            .__amethystate_save_to(&self.store, &self.prefix)
                    }

                    pub fn mutate_lazy(&mut self, f: impl FnOnce(&mut #data_struct_name)) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                        f(&mut self.inner);
                        self.save_lazy()
                    }

                    pub fn mutate(&mut self, f: impl FnOnce(&mut #data_struct_name)) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                        f(&mut self.inner);
                        self.save()
                    }

                    pub fn save(&self) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                        self.save_lazy()?;
                        #crate_name::Store::flush_prefix(&self.store, &self.prefix)
                            .map_err(|why| #crate_name::store::WriteValue::from_store(
                                &self.prefix,
                                ::core::convert::Into::into(why),
                            ))
                    }
                }

                impl #name {
                    pub fn load_with(store: &#crate_name::Store) -> ::core::result::Result<#persisted_struct_name, #crate_name::store::OpenStruct> {
                        Ok(#persisted_struct_name {
                            inner: #data_struct_name::__amethystate_load_from(store, &#prefix_path)?,
                            store: store.clone(),
                            prefix: #prefix_path,
                        })
                    }
                }

                impl #name {
                    pub fn load() -> ::core::result::Result<#persisted_struct_name, #crate_name::store::OpenStruct> {
                        let store = #crate_name::global_store();
                        Self::load_with(&store)
                    }
                }
            }
        }
    };

    let loaded_struct_check = match schema.rules.check.as_ref().map(|at| &at.value) {
        None => quote! {},
        Some(check) => {
            let rule =
                super::unreadable_tokens(crate_name, struct_policy.unwrap_or(OnUnreadable::Refuse));

            quote_spanned! {check.span()=>
                if let ::core::result::Result::Err(__ame_invalid) = #check(&__ame_result, store.context()) {
                    #crate_name::store::refused_struct_or_kept(prefix, __ame_invalid, #rule)?;
                }
            }
        }
    };

    let snapshot_fields = p_fields.iter().map(|field| {
        let fname = &field.ident;
        let ty = &field.ty;

        match field.shape {
            Shape::Node { .. } => quote! { #fname: self.#fname.__ame_to_data() },
            _ => quote! {
                #fname: <#ty as #crate_name::shape::Kind>::snapshot(&self.#fname)
            },
        }
    });

    let snapshot = match mode {
        Mode::Persistent => quote! {},
        Mode::Reactive | Mode::Both => quote! {
            impl #name {
                #[doc(hidden)]
                pub fn __ame_to_data(&self) -> #data_struct_name {
                    #data_struct_name {
                        #(#snapshot_fields,)*
                    }
                }
            }
        },
    };

    let gen_load_save_helpers = !(is_root && matches!(mode, Mode::Reactive));

    let load_save_helpers = if gen_load_save_helpers {
        quote! {
            #[doc(hidden)]
            pub fn __amethystate_load_from(
                store: &#crate_name::Store,
                prefix: &#crate_name::store::StorePath,
            ) -> ::core::result::Result<Self, #crate_name::store::OpenStruct> {
                let __ame_result = Self {
                    #(#store_load_fields,)*
                };
                #loaded_struct_check
                Ok(__ame_result)
            }

            #[doc(hidden)]
            pub fn __amethystate_save_to(
                &self,
                store: &#crate_name::Store,
                prefix: &#crate_name::store::StorePath,
            ) -> ::core::result::Result<(), #crate_name::store::WriteValue> {
                #(#store_save_fields)*
                Ok(())
            }
        }
    } else {
        quote! {}
    };

    quote! {
        #[derive(#crate_name::serde::Serialize, #crate_name::serde::Deserialize, Clone)]
        #(#forwarded_derives)*
        #[serde(crate = #serde_path)]
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub struct #data_struct_name {
            #(#data_fields,)*
        }

        #persistent_wrapper_tokens

        #snapshot

        impl #data_struct_name {
            #load_save_helpers
        }

       impl #crate_name::migration::fields::AmeStateFields for #data_struct_name {
            const FIELDS: &'static [#crate_name::migration::fields::FieldDescriptor] = {
                #[allow(unused_imports)]
                use #crate_name::shape::AnyShape as _;

                #(#flatten_checks)*

                &[
                    #(#field_descriptors),*
                ]
            };
            const VERSION: u32 = #version_val;
            const ID: ::core::option::Option<&'static str> = #id_val;
            const PARENT_PREFIX: #crate_name::store::StaticPath = #prefix_static;

            fn load_struct(ctx: &mut #crate_name::MigrationContext) -> #crate_name::migration::StepResult<Self> {
                Ok(Self {
                    #(#load_fields,)*
                })
            }

            fn save_struct(&self, ctx: &mut #crate_name::MigrationContext) -> #crate_name::migration::StepResult<()> {
                #(#save_fields)*
                Ok(())
            }
        }

        impl #crate_name::AmeState for #name {
            type Data = #data_struct_name;
        }
    }
}
