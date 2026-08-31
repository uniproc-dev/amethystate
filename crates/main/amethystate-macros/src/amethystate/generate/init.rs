use crate::amethystate::generate::{parse_default, path_literal};
use amethystate_macros_core::StoreFieldEntry;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;

pub(crate) fn init_fields(
    crate_name: &TokenStream2,
    entries: &[StoreFieldEntry],
    is_root: bool,
    on_unreadable: Option<&str>,
    on_delete: Option<&str>,
) -> Vec<TokenStream2> {
    entries
        .iter()
        .map(|e| {
            let unreadable = match variant(e.on_unreadable.as_ref())
                .as_deref()
                .or(on_unreadable)
            {
                Some("UseDefault") => quote!(#crate_name::store::OnUnreadable::UseDefault),
                Some(_) => quote!(#crate_name::store::OnUnreadable::Refuse),
                None => quote!(__ame_on_unreadable),
            };

            let deleted = match variant(e.on_delete.as_ref()).as_deref().or(on_delete) {
                Some("UseDefault") => quote!(#crate_name::store::OnDelete::UseDefault),
                Some(_) => quote!(#crate_name::store::OnDelete::Keep),
                None => quote!(__ame_on_delete),
            };

            init_field(crate_name, e, is_root, &unreadable, &deleted)
        })
        .collect::<Vec<_>>()
}

fn variant(written: Option<&syn::Path>) -> Option<String> {
    written
        .and_then(|path| path.segments.last())
        .map(|segment| segment.ident.to_string())
}

fn init_field(
    crate_name: &TokenStream2,
    e: &StoreFieldEntry,
    is_root: bool,
    unreadable: &TokenStream2,
    deleted: &TokenStream2,
) -> TokenStream2 {
    let fname = e.ident.as_ref().unwrap();
    let ty = &e.ty;
    let key = e.stored_name();
    let key_path = path_literal(crate_name, &key);

    if e.nested {
        if is_root {
            quote! {
                #fname: ::std::sync::Arc::new(#ty::new_with_id_under(
                    store,
                    <Self as #crate_name::StateScope>::PATH.join(&#key_path),
                    instance_id,
                    #unreadable,
                    #deleted
                )?)
            }
        } else {
            quote! {
                #fname: ::std::sync::Arc::new(#ty::new_with_id_under(
                    store,
                    namespace.join(&#key_path),
                    instance_id,
                    #unreadable,
                    #deleted
                )?)
            }
        }
    } else if let Some((k, v)) = e.get_map_types() {
        let def = e
            .default
            .as_ref()
            .map(parse_default)
            .unwrap_or_else(|| quote!(::std::collections::HashMap::new()));

        let path_expr = if is_root {
            quote! { <Self as #crate_name::StateScope>::PATH.join(&#key_path) }
        } else {
            quote! { namespace.join(&#key_path) }
        };

        quote! {
            #fname: #crate_name::store::reactive_map_with_path_only::<#k, #v>(
                store,
                #path_expr,
                #def,
                instance_id
            )?
        }
    } else {
        let raw_def = e
            .default
            .as_ref()
            .expect("Default required for leaf fields");
        let def = parse_default(raw_def);

        let path_expr = if is_root {
            quote! { <Self as #crate_name::StateScope>::PATH.join(&#key_path) }
        } else {
            quote! { namespace.join(&#key_path) }
        };

        if e.volatile {
            quote! { #fname: #crate_name::Field::new_volatile_with_id(#path_expr, #def, instance_id) }
        } else {
            let rules = match &e.check {
                Some(check) => quote_spanned! {check.span()=>
                    #crate_name::store::ReadRules::new()
                        .on_unreadable(#unreadable)
                        .on_delete(#deleted)
                        .check(#check)
                },
                None => quote! {
                    #crate_name::store::ReadRules::new()
                        .on_unreadable(#unreadable)
                        .on_delete(#deleted)
                },
            };

            quote! { #fname: #crate_name::store::field_with_path_under(store, #path_expr, #def, instance_id, #rules)? }
        }
    }
}
