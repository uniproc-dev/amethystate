mod accessors;
mod data;
mod init;
mod wasm;

use crate::ts_mapping::map_type_to_ts;
use amethystate_macros_core::{MacroArgs, StoreFieldEntry, get_type_ident_str};
use proc_macro2::{Delimiter, TokenStream as TokenStream2, TokenTree};
use quote::format_ident;
use quote::quote;
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Attribute, Expr, Ident, Token, Visibility};

/// A written path, as a value the compiler has already checked.
///
/// The `const` block is what makes the check a check: `from_static` verifies
/// its two halves in a `const fn`, which only runs at compile time where the
/// call is in a const context. Emitted bare, a path built inside a function
/// body would pay for that walk on every call and fail at first use instead of
/// at the attribute.
pub(crate) fn path_literal(crate_name: &TokenStream2, dotted: &str) -> TokenStream2 {
    let (segments, joined) = path_parts(dotted);

    quote! {
        const { #crate_name::store::StorePath::from_static(&[#(#segments),*], #joined) }
    }
}

pub(crate) const SEPARATOR: char = '.';
pub(crate) const ESCAPE: char = '\\';
pub(crate) const ROOT: &str = ".";

/// The levels a written path names, and the joined form `StorePath` keeps
/// beside them.
///
/// Nothing here knows how a path is joined. Levels come from splitting on the
/// separator, so no level holds one, and the only character the join would have
/// escaped is the escape itself - which [`check_written_path`] refuses. What is
/// left is the identity, so the source string is the joined form, and
/// `StorePath::from_static` checks that claim in a const context rather than
/// taking it.
pub(crate) fn path_parts(dotted: &str) -> (Vec<&str>, String) {
    if dotted.is_empty() || dotted == ROOT {
        return (Vec::new(), String::new());
    }

    let segments: Vec<&str> = dotted.split(SEPARATOR).collect();

    (segments, dotted.to_string())
}

/// The variant an `on_unreadable = ..` names, checked here so a typo is a
/// compile error that lists the two rather than a path that fails to resolve
/// somewhere in generated code.
///
/// The name is taken from the last segment, so `UseDefault` and
/// `OnUnreadable::UseDefault` both write the same thing and neither needs the
/// type in scope.
pub(crate) fn read_policy(written: Option<&syn::Path>) -> Result<Option<String>, syn::Error> {
    named_variant(
        written,
        &["Refuse", "UseDefault"],
        "`Refuse` is what happens without one: construction fails and names the path. `UseDefault` takes the declared default and carries on, leaving the stored value where it is for somebody to fix, with `try_get` saying so until they do",
    )
}

pub(crate) fn delete_policy(written: Option<&syn::Path>) -> Result<Option<String>, syn::Error> {
    named_variant(
        written,
        &["UseDefault", "Keep"],
        "`UseDefault` is what happens without one: the field reports its declared default again. `Keep` goes on reporting the last value it held, which is what a value being drawn wants when something else removed the key",
    )
}

fn named_variant(
    written: Option<&syn::Path>,
    allowed: &[&str],
    hint: &str,
) -> Result<Option<String>, syn::Error> {
    let Some(path) = written else {
        return Ok(None);
    };

    let named = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default();

    if allowed.contains(&named.as_str()) {
        return Ok(Some(named));
    }

    Err(syn::Error::new(
        path.span(),
        format!("`{named}` is not one of these. {hint}"),
    ))
}

/// Whether `ty` is written as `name`, however it is qualified.
pub(crate) fn names_type(ty: &syn::Type, name: &Ident) -> bool {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => path
            .path
            .segments
            .last()
            .is_some_and(|last| &last.ident == name && last.arguments.is_empty()),
        _ => false,
    }
}

pub(crate) fn check_written_path(what: &str, written: &str, root_hint: &str) -> Result<(), String> {
    if written.is_empty() {
        return Err(format!("an empty {what} names no level{root_hint}"));
    }

    if written == ROOT {
        return Err(format!("a {what} of `{ROOT}` names no level{root_hint}"));
    }

    let levels: Vec<&str> = written.split(SEPARATOR).collect();
    let last = levels.len() - 1;

    for (at, level) in levels.iter().enumerate() {
        if level.is_empty() {
            return Err(match at {
                0 => {
                    format!("the {what} starts with `{SEPARATOR}`, so its first level has no name")
                }
                _ if at == last => {
                    format!("the {what} ends with `{SEPARATOR}`, so its last level has no name")
                }
                _ => {
                    format!("the {what} has two `{SEPARATOR}` in a row, with no level between them")
                }
            });
        }

        if level.contains(ESCAPE) {
            return Err(format!(
                "a {what} level cannot hold `{ESCAPE}`, which a path escapes"
            ));
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RpMode {
    Reactive,
    Persistent,
    Both,
}

pub fn generate_code(
    crate_name: TokenStream2,
    vis: &Visibility,
    name: &Ident,
    attrs: &[Attribute],
    prefix: Option<String>,
    entries: &[StoreFieldEntry],
    macro_args: MacroArgs,
) -> TokenStream2 {
    let rp_mode = match macro_args.mode.as_deref() {
        None | Some("reactive") => RpMode::Reactive,
        Some("persistent") => RpMode::Persistent,
        Some("both") => RpMode::Both,
        Some(other) => {
            let err = format!(
                "invalid amethystate mode: \"{}\". Expected one of: \"reactive\", \"persistent\", \"both\"",
                other
            );
            return syn::Error::new(proc_macro2::Span::call_site(), err).to_compile_error();
        }
    };

    if macro_args.target.as_deref() == Some("tauri-wasm") {
        return wasm::generate_wasm_code(crate_name, vis, name, attrs, prefix, entries, macro_args);
    }

    let is_root = prefix.is_some();
    let fields_impl = data::data_impl(
        &crate_name,
        vis,
        name,
        attrs,
        prefix.clone(),
        entries,
        &macro_args,
        rp_mode,
    );

    let struct_fields = accessors::struct_fields(&crate_name, entries);
    let on_unreadable = read_policy(macro_args.on_unreadable.as_ref())
        .ok()
        .flatten();
    let on_delete = delete_policy(macro_args.on_delete.as_ref()).ok().flatten();

    let declared_unreadable = match on_unreadable.as_deref() {
        Some("UseDefault") => {
            quote!(::core::option::Option::Some(#crate_name::store::OnUnreadable::UseDefault))
        }
        Some(_) => quote!(::core::option::Option::Some(#crate_name::store::OnUnreadable::Refuse)),
        None => quote!(::core::option::Option::None),
    };
    let declared_delete = match on_delete.as_deref() {
        Some("UseDefault") => {
            quote!(::core::option::Option::Some(#crate_name::store::OnDelete::UseDefault))
        }
        Some(_) => quote!(::core::option::Option::Some(#crate_name::store::OnDelete::Keep)),
        None => quote!(::core::option::Option::None),
    };

    let nested_checks = entries
        .iter()
        .filter(|e| e.nested)
        .filter(|e| {
            read_policy(e.on_unreadable.as_ref())
                .ok()
                .flatten()
                .or_else(|| on_unreadable.clone())
                .as_deref()
                == Some("Refuse")
        })
        .map(|e| {
            let ty = &e.ty;
            let held = e.stored_name();
            let written = quote!(#ty).to_string();
            let complaint = syn::LitStr::new(
                &format!(
                    "`{name}` declares `on_unreadable = Refuse` over `{held}`, and `{written}` declares `UseDefault` for itself. A struct may demand more than the one holding it and never less: take `UseDefault` off `{written}`, so it inherits, or stop promising `Refuse` here"
                ),
                proc_macro2::Span::call_site(),
            );

            quote! {
                const _: () = assert!(
                    !matches!(
                        <#ty as #crate_name::store::DeclaredPolicy>::ON_UNREADABLE,
                        ::core::option::Option::Some(#crate_name::store::OnUnreadable::UseDefault)
                    ),
                    #complaint
                );
            }
        })
        .collect::<Vec<_>>();

    let policy_impl = quote! {
        impl #crate_name::store::DeclaredPolicy for #name {
            const ON_UNREADABLE: ::core::option::Option<#crate_name::store::OnUnreadable> =
                #declared_unreadable;
            const ON_DELETE: ::core::option::Option<#crate_name::store::OnDelete> =
                #declared_delete;
        }

        #(#nested_checks)*
    };
    let init_fields = init::init_fields(
        &crate_name,
        entries,
        is_root,
        on_unreadable.as_deref(),
        on_delete.as_deref(),
    );
    let node_impl = accessors::node_impl(&crate_name, name, is_root, entries);
    let methods = accessors::methods(&crate_name, entries);
    let scope = accessors::scope(&crate_name, name, prefix.clone());
    let constructor = accessors::constructor(
        &crate_name,
        is_root,
        &init_fields,
        macro_args.check.as_ref(),
        on_unreadable.as_deref(),
    );
    let refused_marker = accessors::refused_marker(entries);

    let schema_export =
        generate_schema_export(&crate_name, name, &prefix, macro_args.version, entries);

    let inherent_subs = if matches!(rp_mode, RpMode::Reactive | RpMode::Both) {
        let sub_all_fields = entries.iter().map(|e| {
            let fname = e.ident.as_ref().unwrap();
            if e.nested {
                quote! {
                    {
                        let cb_clone = cb.clone();
                        scope.watch_scope(self.#fname.subscribe_all(move || cb_clone()));
                    }
                }
            } else if e.get_map_types().is_some() {
                quote! {
                    {
                        let cb_clone = cb.clone();
                        scope.watch(self.#fname.subscribe_any(move |_| cb_clone()));
                    }
                }
            } else {
                quote! {
                    {
                        let cb_clone = cb.clone();
                        scope.watch(self.#fname.subscribe(move |_| cb_clone()));
                    }
                }
            }
        });

        let sub_all_fields_ext = entries.iter().map(|e| {
            let fname = e.ident.as_ref().unwrap();
            if e.nested {
                quote! {
                    {
                        let cb_clone = cb.clone();
                        scope.watch_scope(self.#fname.subscribe_all_external(move || cb_clone()));
                    }
                }
            } else {
                quote! {
                    {
                        let cb_clone = cb.clone();
                        scope.watch(self.#fname.subscription_with().external().register(move |_| cb_clone()));
                    }
                }
            }
        });

        quote! {
            pub fn subscribe_all<F>(&self, callback: F) -> #crate_name::ReactiveScope
            where
                F: Fn() + Send + Sync + 'static,
            {
                let cb = ::std::sync::Arc::new(callback);
                let mut scope = #crate_name::ReactiveScope::new();

                #(#sub_all_fields)*

                scope
            }

            pub fn subscribe_all_external<F>(&self, callback: F) -> #crate_name::ReactiveScope
            where
                F: Fn() + Send + Sync + 'static,
            {
                let cb = ::std::sync::Arc::new(callback);
                let mut scope = #crate_name::ReactiveScope::new();

                #(#sub_all_fields_ext)*

                scope
            }
        }
    } else {
        quote! {}
    };

    let slice_impl = if is_root {
        let load_fn = match rp_mode {
            RpMode::Persistent => quote! { Self::load_with(store) },
            _ => quote! { Self::new_with(store) },
        };

        let slice_subs = if matches!(rp_mode, RpMode::Reactive | RpMode::Both) {
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
                fn load_slice(store: &#crate_name::Store) -> #crate_name::StorageResult<Self> {
                    #load_fn
                }

                #slice_subs
            }
        }
    } else {
        quote! {}
    };

    let global_new_impl = if is_root {
        quote! {
            impl #name {
                pub fn new() -> #crate_name::StorageResult<Self> {
                    let store = #crate_name::global_store();
                    Self::new_with(&store)
                }
            }
        }
    } else {
        quote! {}
    };

    let fork_fields = entries.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        if e.nested {
            quote! { #fname: ::std::sync::Arc::new(self.#fname.fork_with_id(new_id)) }
        } else {
            quote! { #fname: self.#fname.fork_with_id(new_id) }
        }
    });

    let fork_impl = if matches!(rp_mode, RpMode::Reactive | RpMode::Both) {
        quote! {
            pub fn fork(&self) -> Self {
                self.fork_with_id(#crate_name::uuid::Uuid::new_v4())
            }

            #[doc(hidden)]
            pub fn fork_with_id(&self, new_id: #crate_name::uuid::Uuid) -> Self {
                Self {
                    __amethystate_instance_id: #crate_name::observability::InstanceGuard::new(
                        new_id,
                        ::std::any::type_name::<Self>(),
                    ),
                    #(#fork_fields,)*
                }
            }
        }
    } else {
        quote! {}
    };

    let debug_fields_auto = entries.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        quote! { .field(stringify!(#fname), (&__AmeW(&self.#fname)).__ame()) }
    });

    match rp_mode {
        RpMode::Reactive | RpMode::Both => {
            quote! {
                #[derive(Clone)]
                #(#attrs)* #vis struct #name {
                    __amethystate_instance_id: ::std::sync::Arc<#crate_name::observability::InstanceGuard>,
                    #(#struct_fields,)*
                }

                impl ::std::fmt::Debug for #name {
                    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                        struct __AmeOpaque;
                        impl ::std::fmt::Debug for __AmeOpaque {
                            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                                f.write_str("<opaque>")
                            }
                        }
                        struct __AmeW<'a, T>(&'a T);
                        trait __AmeViaDebug {
                            fn __ame(&self) -> &dyn ::std::fmt::Debug;
                        }
                        impl<'a, T: ::std::fmt::Debug> __AmeViaDebug for __AmeW<'a, T> {
                            fn __ame(&self) -> &dyn ::std::fmt::Debug { self.0 }
                        }
                        trait __AmeViaFallback {
                            fn __ame(&self) -> &dyn ::std::fmt::Debug;
                        }
                        impl<'a, T> __AmeViaFallback for &__AmeW<'a, T> {
                            fn __ame(&self) -> &dyn ::std::fmt::Debug { &__AmeOpaque }
                        }
                        f.debug_struct(stringify!(#name))
                            #(#debug_fields_auto)*
                            .finish()
                    }
                }
                #scope
                impl #name {
                    #constructor #(#methods)*
                    #refused_marker
                    #fork_impl
                    #inherent_subs
                }
                #global_new_impl
                #node_impl
                #fields_impl
                #schema_export
                #slice_impl
                #policy_impl
            }
        }
        RpMode::Persistent => {
            quote! {
                #scope
                #fields_impl
                #schema_export
                #slice_impl
                #policy_impl
            }
        }
    }
}

fn generate_schema_export(
    crate_name: &TokenStream2,
    name: &Ident,
    prefix: &Option<String>,
    version: Option<u32>,
    entries: &[StoreFieldEntry],
) -> TokenStream2 {
    let struct_name_str = name.to_string();
    let prefix_tokens = match prefix {
        Some(p) => {
            let path = path_literal(crate_name, p);
            quote! { Some(#path) }
        }
        None => quote! { None },
    };
    let exported_prefix_tokens = match prefix {
        Some(p) => quote! { Some(#p) },
        None => quote! { None },
    };

    let field_metas = entries.iter().map(|e| {
        let fname = e.ident.as_ref().unwrap();
        let fname_str = fname.to_string();
        let (ts_type, full_ts_type) = map_type_to_ts(e.ty.clone());

        let ty = &e.ty;
        let rust_type_str = quote!(#ty).to_string();

        let kind_tokens = if e.volatile {
            quote! { #crate_name::tauri::FieldKind::Volatile }
        } else if e.nested {
            let sname = get_type_ident_str(&e.ty);
            quote! { #crate_name::tauri::FieldKind::Nested { struct_name: #sname } }
        } else if let Some((k, v)) = e.get_map_types() {
            let k_ts = map_type_to_ts(k.clone()).1;
            let v_ts = map_type_to_ts(v.clone()).1;
            let k_rust = quote!(#k).to_string();
            let v_rust = quote!(#v).to_string();
            quote! {
                #crate_name::tauri::FieldKind::ReactiveMap {
                    key_type: #k_ts,
                    value_type: #v_ts,
                    key_rust_type: #k_rust,
                    value_rust_type: #v_rust,
                }
            }
        } else {
            quote! { #crate_name::tauri::FieldKind::Plain }
        };

        quote! {
            #crate_name::tauri::FieldExportMeta {
                name: #fname_str,
                ts_type: #ts_type,
                full_ts_type: #full_ts_type,
                rust_type: #rust_type_str,
                kind: #kind_tokens,
            }
        }
    });

    let data_struct_name = format_ident!("{}_Data", name);
    let version_val = version.unwrap_or(0);

    let schema_entry = quote! {
        #crate_name::inventory::submit! {
            #crate_name::observability::SchemaEntry {
                prefix: #prefix_tokens,
                struct_name: #struct_name_str,
                version: #version_val,
                schema_hash: <#data_struct_name as #crate_name::migration::types::AmeType>::TYPE_HASH,
                fields: <#data_struct_name as #crate_name::migration::fields::AmeStateFields>::FIELDS,
            }
        }
    };

    let tauri_entry = if cfg!(feature = "tauri") {
        quote! {
            #crate_name::inventory::submit! {
                #crate_name::tauri::SchemaExportEntry {
                    prefix: #exported_prefix_tokens,
                    struct_name: #struct_name_str,
                    fields: &[
                        #(#field_metas),*
                    ],
                }
            }
        }
    } else {
        quote!()
    };

    quote! {
        #schema_entry
        #tauri_entry
    }
}

struct MapEntry {
    key: Expr,
    _colon: Token![:],
    value: Expr,
}

impl Parse for MapEntry {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(MapEntry {
            key: input.parse()?,
            _colon: input.parse()?,
            value: input.parse()?,
        })
    }
}

pub(crate) fn parse_default(tokens: &TokenStream2) -> TokenStream2 {
    let mut iter = tokens.clone().into_iter();

    match iter.next() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket => {
            let content = g.stream();
            quote! { vec![#content] }
        }
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            let content = g.stream();

            if content.is_empty() {
                return quote! { ::std::collections::HashMap::default() };
            }

            let parser = Punctuated::<MapEntry, Token![,]>::parse_terminated;
            if let Ok(pairs) = parser.parse2(content)
                && !pairs.is_empty()
            {
                let inserts = pairs.iter().map(|pair| {
                    let k = &pair.key;
                    let v = &pair.value;
                    quote! { __map.insert(::std::convert::Into::into(#k), #v); }
                });

                return quote! {
                    {
                        let mut __map = ::std::collections::HashMap::default();
                        #( #inserts )*
                        __map
                    }
                };
            }

            tokens.clone()
        }
        _ => tokens.clone(),
    }
}
