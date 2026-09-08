//! What this struct tells the rest of the process about itself: the entries a
//! running program can walk to find every schema that was declared.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

use super::path_literal;
use crate::amethystate::model::{Placement, Schema, Shape};
use crate::ts_mapping::map_type_to_ts;
use amethystate_macros_core::get_type_ident_str;

/// The inventory entries for this schema: one the store reads to know what
/// was declared where, and one a Tauri front end reads to write types for it.
pub(crate) fn entries(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let named = schema.name.to_string();
    let prefix = schema.prefix.as_ref().map(Placement::path);

    let at = match &prefix {
        Some(written) => {
            let path = path_literal(crate_name, written);
            quote! { Some(#path) }
        }
        None => quote! { None },
    };

    let data_struct_name = format_ident!("{}_Data", schema.name);
    let version = schema.version;

    let declared = quote! {
        #crate_name::inventory::submit! {
            #crate_name::schema::SchemaEntry {
                prefix: #at,
                struct_name: #named,
                version: #version,
                fields: <#data_struct_name as #crate_name::migration::fields::AmeStateFields>::FIELDS,
            }
        }
    };

    let for_tauri = tauri_entry(crate_name, schema, prefix.as_deref());

    quote! {
        #declared
        #for_tauri
    }
}

fn tauri_entry(crate_name: &TokenStream2, schema: &Schema, prefix: Option<&str>) -> TokenStream2 {
    if !cfg!(feature = "tauri") {
        return quote!();
    }

    let named = schema.name.to_string();
    let at = match prefix {
        Some(written) => quote! { Some(#written) },
        None => quote! { None },
    };

    let each = schema.fields.iter().map(|field| {
        let fname_str = field.ident.to_string();
        let (ts_type, full_ts_type) = map_type_to_ts(field.ty.clone());

        let ty = &field.ty;
        let rust_type_str = quote!(#ty).to_string();

        let kind = match &field.shape {
            Shape::Volatile { .. } => quote! { #crate_name::tauri::FieldKind::Volatile },
            Shape::Node { .. } => {
                let sname = get_type_ident_str(&field.ty);
                quote! { #crate_name::tauri::FieldKind::Nested { struct_name: #sname } }
            }
            Shape::Map { key, value, .. } => {
                let k_ts = map_type_to_ts((**key).clone()).1;
                let v_ts = map_type_to_ts((**value).clone()).1;
                let k_rust = quote!(#key).to_string();
                let v_rust = quote!(#value).to_string();
                quote! {
                    #crate_name::tauri::FieldKind::ReactiveMap {
                        key_type: #k_ts,
                        value_type: #v_ts,
                        key_rust_type: #k_rust,
                        value_rust_type: #v_rust,
                    }
                }
            }
            Shape::Leaf { .. } => quote! { #crate_name::tauri::FieldKind::Plain },
        };

        quote! {
            #crate_name::tauri::FieldExportMeta {
                name: #fname_str,
                ts_type: #ts_type,
                full_ts_type: #full_ts_type,
                rust_type: #rust_type_str,
                kind: #kind,
            }
        }
    });

    quote! {
        #crate_name::inventory::submit! {
            #crate_name::tauri::SchemaExportEntry {
                prefix: #at,
                struct_name: #named,
                fields: &[
                    #(#each),*
                ],
            }
        }
    }
}
