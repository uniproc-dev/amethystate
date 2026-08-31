use quote::quote;
use syn::{
    FnArg, Ident, ItemFn, PatType, ReturnType, Token, Type,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
};

use crate::amethystate::amethystate_crate_path;

struct RenameMeta {
    old: Ident,
    _arrow: Token![=>],
    new: Ident,
}

impl Parse for RenameMeta {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            old: input.parse()?,
            _arrow: input.parse()?,
            new: input.parse()?,
        })
    }
}

pub fn migrate_impl(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let item_fn = parse_macro_input!(input as ItemFn);
    match migrate_impl_inner(args, item_fn) {
        Ok(ts) => ts,
        Err(e) => e.to_compile_error().into(),
    }
}

/// Whether the step opts out of being found through the linker.
fn wants_explicit(args: proc_macro::TokenStream) -> syn::Result<bool> {
    if args.is_empty() {
        return Ok(false);
    }

    let args: proc_macro2::TokenStream = args.into();
    let idents = syn::parse::Parser::parse2(
        Punctuated::<Ident, Token![,]>::parse_terminated,
        args.clone(),
    )
    .map_err(|_| syn::Error::new_spanned(&args, "expected `explicit`"))?;

    for ident in &idents {
        if ident != "explicit" {
            return Err(syn::Error::new_spanned(
                ident,
                format!("unknown option `{ident}`, expected `explicit`"),
            ));
        }
    }

    Ok(!idents.is_empty())
}

pub fn migrate_impl_inner(
    args: proc_macro::TokenStream,
    mut item_fn: ItemFn,
) -> syn::Result<proc_macro::TokenStream> {
    let crate_name = amethystate_crate_path();
    let explicit = wants_explicit(args)?;

    let fn_name = &item_fn.sig.ident;
    let description = fn_name.to_string();

    let mut inputs = item_fn.sig.inputs.iter();
    let first_arg = inputs.next().unwrap();

    let old_ty = match first_arg {
        FnArg::Typed(PatType { ty, .. }) => ty.clone(),
        _ => {
            return Err(syn::Error::new_spanned(
                &item_fn.sig,
                "first argument must be `AmeData<T>`, e.g. `AmeData<v1::Config>`",
            ));
        }
    };

    let has_ctx = inputs.next().is_some();

    let new_ty = match &item_fn.sig.output {
        ReturnType::Type(_, ty) => match extract_target_type(ty) {
            Some(ty) => ty,
            None => {
                return Err(syn::Error::new_spanned(
                    ty,
                    "return type must be `MigrationResult<AmeData<T>>`, e.g. `MigrationResult<AmeData<Config>>`",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &item_fn.sig,
                "return type must be `MigrationResult<AmeData<T>>`, e.g. `MigrationResult<AmeData<Config>>`",
            ));
        }
    };

    let _old_inner_ty = extract_ame_data_inner(&old_ty).ok_or_else(|| {
        syn::Error::new_spanned(
            &old_ty,
            "first argument must be `AmeData<T>`, e.g. `AmeData<v1::Config>`",
        )
    })?;

    let new_inner_ty = extract_ame_data_inner(&new_ty)
        .ok_or_else(|| syn::Error::new_spanned(
            &new_ty,
            "return type must be `MigrationResult<AmeData<T>>`, e.g. `MigrationResult<AmeData<Config>>`"
        ))?;

    let mut renames = Vec::new();
    let mut cleaned_attrs = Vec::new();

    for attr in item_fn.attrs.drain(..) {
        if attr.path().is_ident("rename") {
            let parsed = attr
                .parse_args_with(Punctuated::<RenameMeta, Token![,]>::parse_terminated)
                .unwrap();
            renames.extend(parsed);
        } else {
            cleaned_attrs.push(attr);
        }
    }
    item_fn.attrs = cleaned_attrs;

    let check_fields = {
        let old_fields = renames.iter().map(|r| &r.old);
        let new_fields = renames.iter().map(|r| &r.new);
        quote! {
            const _: () = {
                #[allow(dead_code, clippy::no_effect, unused_variables)]
                fn _check_fields(old: &#old_ty, new: &#new_ty) {
                    #(
                        let _ = &old.#old_fields;
                        let _ = &new.#new_fields;
                    )*
                }
            };
        }
    };

    let rename_tuples = renames.iter().map(|r| {
        let old_str = r.old.to_string();
        let new_str = r.new.to_string();
        quote! { (#old_str, #new_str) }
    });

    let call_expr = if has_ctx {
        quote! { #fn_name(old_val, ctx_val) }
    } else {
        quote! { #fn_name(old_val) }
    };

    let impl_block = quote! {
        impl #crate_name::migration::migrate_from::MigrateFrom<#old_ty> for #new_ty {
            const RENAMES: &'static [(&'static str, &'static str)] = &[
                #(#rename_tuples),*
            ];

            fn migrate(old_val: #old_ty, ctx_val: &mut #crate_name::migration::MigrationContext) -> #crate_name::StorageResult<Self> {
                #call_expr
            }
        }
    };

    let struct_name = if let Type::Path(tp) = &new_inner_ty {
        tp.path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .ok_or_else(|| syn::Error::new_spanned(&new_inner_ty, "expected a type path"))?
    } else {
        return Err(syn::Error::new_spanned(
            &new_inner_ty,
            "expected a type path",
        ));
    };

    let entry = quote! {
        #crate_name::migration::registry::MigrationStepEntry {
                prefix: <#new_ty as #crate_name::migration::fields::AmeStateFields>::PARENT_PREFIX,
                target_version: <#new_ty as #crate_name::migration::fields::AmeStateFields>::VERSION,
                dependencies: <#new_ty as #crate_name::migration::fields::AmeStateFields>::MIGRATION_DEPS,
                description: #description,
                struct_name: #struct_name,
                schema_hash: <#new_ty as #crate_name::migration::fields::AmeStateFields>::SCHEMA_HASH,
                fields: <#new_ty as #crate_name::migration::fields::AmeStateFields>::FIELDS,
                run: |ctx| {
                    use #crate_name::migration::fields::AmeStateFields;
                    use #crate_name::migration::migrate_from::MigrateFrom;

                    let old_data = <#old_ty as AmeStateFields>::load_struct(ctx)?;
                    let new_data = <#new_ty as MigrateFrom<#old_ty>>::migrate(old_data, ctx)?;

                    for field in <#old_ty as AmeStateFields>::FIELDS {
                        let is_renamed = <#new_ty as MigrateFrom<#old_ty>>::RENAMES
                            .iter()
                            .any(|(old_k, _)| *old_k == field.name);
                        let is_kept = <#new_ty as AmeStateFields>::FIELDS
                            .iter()
                            .any(|f| f.name == field.name);

                        if is_renamed || !is_kept {
                            ctx.delete(field.name)?;
                        }
                    }

                    new_data.save_struct(ctx)?;
                    Ok(())
                }
        }
    };

    let registration = if explicit {
        let const_name = Ident::new(
            &fn_name.to_string().to_uppercase(),
            proc_macro2::Span::call_site(),
        );
        quote! {
            #[allow(non_upper_case_globals)]
            pub const #const_name: #crate_name::migration::registry::MigrationStepEntry = #entry;
        }
    } else {
        quote! {
            #crate_name::inventory::submit! { #entry }
        }
    };

    Ok(quote! {
        #item_fn
        #check_fields
        #impl_block
        #registration
    }
    .into())
}

fn extract_ame_data_inner(ty: &Type) -> Option<Type> {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "AmeData"
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first()
    {
        return Some(inner_ty.clone());
    }
    None
}

fn extract_target_type(ty: &Type) -> Option<Type> {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "MigrationResult"
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first()
    {
        return Some(inner_ty.clone());
    }

    None
}
