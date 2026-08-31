use crate::amethystate::generate::path_parts;
use amethystate_macros_core::StoreFieldEntry;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::Ident;
use syn::spanned::Spanned;

/// The stored type of one field, as it appears in the reactive struct.
pub(crate) fn field_type(crate_name: &TokenStream2, e: &StoreFieldEntry) -> TokenStream2 {
    let ty = &e.ty;

    if e.nested {
        quote! { ::std::sync::Arc<#ty> }
    } else if let Some((k, v)) = e.get_map_types() {
        quote! { #crate_name::ReactiveMap<#k, #v> }
    } else {
        quote! { #crate_name::Field<#ty> }
    }
}

pub(crate) fn struct_fields<'a>(
    crate_name: &'a TokenStream2,
    entries: &'a [StoreFieldEntry],
) -> impl Iterator<Item = TokenStream2> + 'a {
    entries.iter().map(move |e| {
        let fname = e.ident.as_ref().unwrap();
        let fvis = &e.vis;
        let ty = field_type(crate_name, e);

        quote! { #fvis #fname: #ty }
    })
}

pub(crate) fn methods<'a>(
    crate_name: &'a TokenStream2,
    entries: &'a [StoreFieldEntry],
) -> impl Iterator<Item = TokenStream2> + 'a {
    entries.iter().map(move |e| {
        let fname = e.ident.as_ref().unwrap();
        let ty = &e.ty;

        if e.nested {
            quote! { pub fn #fname(&self) -> ::std::sync::Arc<#ty> { self.#fname.clone() } }
        } else if let Some((k, v)) = e.get_map_types() {
            quote! {
                pub fn #fname(&self) -> #crate_name::ReactiveMap<#k, #v> {
                    self.#fname.clone()
                }
            }
        } else {
            quote! {
                pub fn #fname(&self) -> #crate_name::Field<#ty> {
                    self.#fname.clone()
                }
            }
        }
    })
}

/// The types this struct's constructor always constructs in turn.
///
/// A `nested` field is built unconditionally, so those are the edges a cycle
/// can run along. Nothing else is: a map recursing through its value type
/// decodes those values rather than constructing them.
fn construction_edges(crate_name: &TokenStream2, entries: &[StoreFieldEntry]) -> Vec<TokenStream2> {
    entries
        .iter()
        .filter(|e| e.nested)
        .map(|e| {
            let ty = &e.ty;
            quote_spanned! {ty.span()=>
                let _: () = <#ty as #crate_name::AmeStateNode>::CONSTRUCTION_TERMINATES;
            }
        })
        .collect()
}

pub(crate) fn node_impl(
    crate_name: &TokenStream2,
    name: &Ident,
    is_root: bool,
    entries: &[StoreFieldEntry],
) -> TokenStream2 {
    let edges = construction_edges(crate_name, entries);
    let terminates = quote! {
        const CONSTRUCTION_TERMINATES: () = { #(#edges)* };
    };
    let force = quote! {
        const _: () = <#name as #crate_name::AmeStateNode>::CONSTRUCTION_TERMINATES;
    };

    if is_root {
        quote! {
            impl #crate_name::AmeStateNode for #name {
                #terminates

                fn new_node(store: &#crate_name::Store, _path: &#crate_name::store::StorePath) -> #crate_name::StorageResult<Self> {
                    Self::new_with(store)
                }

                fn new_node_with_id(store: &#crate_name::Store, _path: &#crate_name::store::StorePath, instance_id: #crate_name::uuid::Uuid) -> #crate_name::StorageResult<Self> {
                    Self::new_with_id(store, instance_id)
                }
            }

            #force
        }
    } else {
        quote! {
            impl #crate_name::AmeStateNode for #name {
                #terminates

                fn new_node(store: &#crate_name::Store, path: &#crate_name::store::StorePath) -> #crate_name::StorageResult<Self> {
                    Self::new(store, path)
                }

                fn new_node_with_id(store: &#crate_name::Store, path: &#crate_name::store::StorePath, instance_id: #crate_name::uuid::Uuid) -> #crate_name::StorageResult<Self> {
                    Self::new_with_id(store, path, instance_id)
                }
            }

            #force
        }
    }
}

pub(crate) fn scope(
    crate_name: &TokenStream2,
    name: &Ident,
    prefix: Option<String>,
) -> Option<TokenStream2> {
    prefix.map(|p| {
        let (segments, joined) = path_parts(&p);
        quote! {
            impl #crate_name::StateScope for #name {
                const PATH: #crate_name::store::StorePath =
                    #crate_name::store::StorePath::from_static(&[#(#segments),*], #joined);
                const KEY: &'static str = #joined;
            }
        }
    })
}

/// Marks the fields a struct's own check named, so each of them answers
/// `try_get` with what the check said.
///
/// A nested field is marked all the way down: what failed is a relationship
/// the holder declared, and nothing inside the nested struct can be told apart
/// by it.
pub(crate) fn refused_marker(entries: &[StoreFieldEntry]) -> TokenStream2 {
    let marks = entries.iter().filter(|e| e.get_map_types().is_none()).map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let named = e.stored_name();

        let mark = if e.nested {
            quote! { self.#fname.__ame_refused(::core::option::Option::None, why); }
        } else {
            quote! { self.#fname.__ame_refused(why); }
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
fn struct_check(
    crate_name: &TokenStream2,
    is_root: bool,
    check: Option<&syn::Path>,
    on_unreadable: Option<&str>,
) -> TokenStream2 {
    let Some(check) = check else {
        return quote! {};
    };

    let rule = match on_unreadable {
        Some("UseDefault") => quote!(#crate_name::store::OnUnreadable::UseDefault),
        Some(_) => quote!(#crate_name::store::OnUnreadable::Refuse),
        None => quote!(__ame_on_unreadable),
    };

    let where_it_is = if is_root {
        quote! { &<Self as #crate_name::StateScope>::PATH }
    } else {
        quote! { &namespace }
    };

    quote_spanned! {check.span()=>
        if let ::core::result::Result::Err(__ame_invalid) = #check(&result, store.context()) {
            match #rule {
                #crate_name::store::OnUnreadable::Refuse => {
                    return ::core::result::Result::Err(
                        #crate_name::store::refused_under(#where_it_is, &__ame_invalid)
                    );
                }
                #crate_name::store::OnUnreadable::UseDefault => {
                    result.__ame_refused(__ame_invalid.fields(), __ame_invalid.reason());
                }
            }
        }
    }
}

pub(crate) fn constructor(
    crate_name: &TokenStream2,
    is_root: bool,
    init_fields: &[TokenStream2],
    check: Option<&syn::Path>,
    on_unreadable: Option<&str>,
) -> TokenStream2 {
    let checked = struct_check(crate_name, is_root, check, on_unreadable);
    if is_root {
        quote! {
            pub fn new_with(store: &#crate_name::Store) -> #crate_name::StorageResult<Self> {
                Self::new_with_id(store, #crate_name::uuid::Uuid::new_v4())
            }

            pub fn new_with_id(store: &#crate_name::Store, instance_id: #crate_name::uuid::Uuid) -> #crate_name::StorageResult<Self> {
                Self::new_with_id_under(
                    store,
                    instance_id,
                    ::std::default::Default::default(),
                    ::std::default::Default::default(),
                )
            }

            /// The same, told what the struct holding this one decided about a
            /// value it cannot read and a key removed under it.
            ///
            /// Whatever this struct declared for itself wins; these are what a
            /// field falls back to when neither it nor this struct said.
            pub fn new_with_id_under(
                store: &#crate_name::Store,
                instance_id: #crate_name::uuid::Uuid,
                __ame_on_unreadable: #crate_name::store::OnUnreadable,
                __ame_on_delete: #crate_name::store::OnDelete,
            ) -> #crate_name::StorageResult<Self> {
                use #crate_name::{StoreBackend, StoreExt};
                let __amethystate_guard = #crate_name::observability::InstanceGuard::new(
                    instance_id,
                    ::std::any::type_name::<Self>(),
                );
                let result = Self { __amethystate_instance_id: __amethystate_guard, #(#init_fields,)* };
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
            ) -> #crate_name::StorageResult<Self> {
                Self::new_with_id(store, namespace, #crate_name::uuid::Uuid::new_v4())
            }

            pub fn new_with_id(
                store: &#crate_name::Store,
                namespace: impl #crate_name::store::IntoStorePath,
                instance_id: #crate_name::uuid::Uuid,
            ) -> #crate_name::StorageResult<Self> {
                Self::new_with_id_under(
                    store,
                    namespace,
                    instance_id,
                    ::std::default::Default::default(),
                    ::std::default::Default::default(),
                )
            }

            /// The same, told what the struct holding this one decided about a
            /// value it cannot read and a key removed under it.
            ///
            /// Whatever this struct declared for itself wins; these are what a
            /// field falls back to when neither it nor this struct said.
            pub fn new_with_id_under(
                store: &#crate_name::Store,
                namespace: impl #crate_name::store::IntoStorePath,
                instance_id: #crate_name::uuid::Uuid,
                __ame_on_unreadable: #crate_name::store::OnUnreadable,
                __ame_on_delete: #crate_name::store::OnDelete,
            ) -> #crate_name::StorageResult<Self> {
                use #crate_name::{StoreBackend, StoreExt};
                let namespace = #crate_name::store::to_path(namespace)?;
                let __amethystate_guard = #crate_name::observability::InstanceGuard::new(
                    instance_id,
                    ::std::any::type_name::<Self>(),
                );
                let result = Self { __amethystate_instance_id: __amethystate_guard, #(#init_fields,)* };
                #checked
                store.mark_initialized(&namespace)?;
                Ok(result)
            }
        }
    }
}
