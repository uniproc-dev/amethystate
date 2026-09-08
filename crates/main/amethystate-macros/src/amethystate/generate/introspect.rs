//! Listing a struct's own fields at runtime, without their types.
//!
//! Everything a view needs is generated beside it rather than looked up in
//! `AmeStateFields::FIELDS`: that array is sorted by name and leaves `volatile`
//! fields out, so an index into it is not an index into the declaration.
//!
//! Indexed rather than collected, so listing costs nothing until somebody
//! walks it.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use super::reactive::shown_value;
use crate::amethystate::model::{Schema, Shape};

pub(crate) fn inspect(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if !schema.mode.watches() {
        return quote! {};
    }

    let name = &schema.name;
    let count = schema.fields.len();

    let arms = schema.fields.iter().enumerate().map(|(index, field)| {
        let fname = &field.ident;
        let declared = fname.to_string();
        let ty = &field.ty;
        let type_name = quote!(#ty).to_string().replace(" ", "");

        // A leaf hands its value back by value, so it is rendered here rather
        // than borrowed. A map says how many it holds - printing ninety
        // thousand entries is not showing anything. A nested struct says
        // nothing: the walk descends into it.
        let shown = match field.shape {
            Shape::Node { .. } => quote! { ::std::string::String::new() },
            Shape::Map { .. } => quote! {
                format!("{} entries", self.#fname.len())
            },
            _ => {
                let held = shown_value(crate_name, quote! { &__ame_held });
                quote! {
                    {
                        let __ame_held = self.#fname.get();
                        format!("{:?}", #held)
                    }
                }
            }
        };

        let role = match field.shape {
            Shape::Node { .. } => quote! { Role::Node },
            Shape::Map { .. } => quote! { Role::Map },
            _ => quote! { Role::Field },
        };

        // Where it is. A leaf and a map know their own path; a nested struct
        // is asked for the one it was built under, and a volatile field has
        // none - its path is the one it would have had.
        let at = match field.shape {
            Shape::Node { .. } => quote! {
                #crate_name::observability::Inspect::at(&*self.#fname)
            },
            _ => quote! { self.#fname.path().clone() },
        };

        let inside = match field.shape {
            Shape::Node { .. } => quote! {
                ::core::option::Option::Some(
                    &*self.#fname as &dyn #crate_name::observability::Inspect,
                )
            },
            _ => quote! { ::core::option::Option::None },
        };

        // Only a leaf holds one value the store can disagree about. A map's
        // entries are its own business, and a nested struct answers for its
        // fields when the walk descends.
        let disagreement = match field.shape {
            Shape::Leaf { .. } => quote! { self.#fname.__ame_disagreement() },
            _ => quote! { ::core::option::Option::None },
        };

        let stored = &field.stored.value;
        let described = &field.described;
        let volatile = matches!(field.shape, Shape::Volatile { .. });

        quote! {
            #index => ::core::option::Option::Some(
                #crate_name::observability::FieldView {
                    declared: #declared,
                    stored: #stored,
                    at: #at,
                    type_name: #type_name,
                    described: #described,
                    role: #role,
                    volatile: #volatile,
                    shown: #shown,
                    disagreement: #disagreement,
                    inside: #inside,
                }
            ),
        }
    });

    quote! {
        impl #crate_name::observability::Inspect for #name {
            fn at(&self) -> #crate_name::store::StorePath {
                self.__amethystate_at.clone()
            }

            fn field_count(&self) -> usize {
                #count
            }

            fn field_at(&self, index: usize) -> ::core::option::Option<
                #crate_name::observability::FieldView<'_>
            > {
                use #crate_name::migration::fields::Role;
                use #crate_name::observability::{ShownAsOpaque, ShownByDebug};

                match index {
                    #(#arms)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    }
}
