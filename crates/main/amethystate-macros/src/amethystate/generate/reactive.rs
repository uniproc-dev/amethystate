//! What only the watching half of a struct has: subscriptions over every
//! field at once, a copy under a new identity, and a `Debug` that prints what
//! it can.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use super::accessors;
use crate::amethystate::model::{Schema, Shape};

/// The struct callers hold: an identity, and one watching field per declared
/// field.
///
/// The identity is what tells a change this handle made from one another
/// handle made, so it is part of the struct rather than of any field.
pub(crate) fn declaration(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if !schema.mode.watches() {
        return quote! {};
    }

    let (vis, name) = (&schema.vis, &schema.name);
    let attrs = super::forwarded_without(&schema.forwarded, &["Clone", "Debug"]);
    let fields = accessors::struct_fields(crate_name, &schema.fields);

    quote! {
        #[derive(Clone)]
        #(#attrs)* #vis struct #name {
            __amethystate_instance_id: ::std::sync::Arc<#crate_name::store::instances::InstanceGuard>,
            /// Where this struct's fields hang, kept so it can say so: a root
            /// knows it from `StateScope`, and one that is embedded is only
            /// told at the call that built it.
            __amethystate_at: #crate_name::store::StorePath,
            #(#fields,)*
        }
    }
}

/// Everything callers reach through the struct itself.
pub(crate) fn inherent(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if !schema.mode.watches() {
        return quote! {};
    }

    let name = &schema.name;
    let constructor = accessors::constructor(crate_name, schema);
    let methods = accessors::methods(crate_name, schema);
    let refused = accessors::refused_marker(schema);
    let forking = fork(crate_name, schema);
    let watching = subscriptions(crate_name, schema);

    quote! {
        impl #name {
            #constructor
            #methods
            #refused
            #forking
            #watching
        }
    }
}

/// `subscribe_all` and `subscribe_all_external`, which reach every field
/// including the ones inside nested structs.
pub(crate) fn subscriptions(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if !schema.mode.watches() {
        return quote! {};
    }

    let fields = &schema.fields;

    let each = fields.iter().map(|field| {
        let fname = &field.ident;
        match field.shape {
            Shape::Node { .. } => quote! {
                {
                    let cb_clone = cb.clone();
                    scope.watch_scope(self.#fname.subscribe_all(move || cb_clone()));
                }
            },
            Shape::Map { .. } => quote! {
                {
                    let cb_clone = cb.clone();
                    scope.watch(self.#fname.subscribe_any(move |_| cb_clone()));
                }
            },
            Shape::Leaf { .. } | Shape::Volatile { .. } => quote! {
                {
                    let cb_clone = cb.clone();
                    scope.watch(self.#fname.subscribe(move |_| cb_clone()));
                }
            },
        }
    });

    let each_external = fields.iter().map(|field| {
        let fname = &field.ident;
        match field.shape {
            Shape::Node { .. } => quote! {
                {
                    let cb_clone = cb.clone();
                    scope.watch_scope(self.#fname.subscribe_all_external(move || cb_clone()));
                }
            },
            _ => quote! {
                {
                    let cb_clone = cb.clone();
                    scope.watch(self.#fname.subscription_with().external().register(move |_| cb_clone()));
                }
            },
        }
    });

    quote! {
        pub fn subscribe_all<F>(&self, callback: F) -> #crate_name::ReactiveScope
        where
            F: Fn() + Send + Sync + 'static,
        {
            let cb = ::std::sync::Arc::new(callback);
            let mut scope = #crate_name::ReactiveScope::new();

            #(#each)*

            scope
        }

        pub fn subscribe_all_external<F>(&self, callback: F) -> #crate_name::ReactiveScope
        where
            F: Fn() + Send + Sync + 'static,
        {
            let cb = ::std::sync::Arc::new(callback);
            let mut scope = #crate_name::ReactiveScope::new();

            #(#each_external)*

            scope
        }
    }
}

/// A copy of this struct under a new identity.
///
/// Every field forks in turn, so what comes back watches the same paths and
/// answers to a different id - which is how a change made through one handle
/// is told apart from a change made through another.
pub(crate) fn fork(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if !schema.mode.watches() {
        return quote! {};
    }

    let each = schema.fields.iter().map(|field| {
        let fname = &field.ident;
        match field.shape {
            Shape::Node { .. } => {
                quote! { #fname: ::std::sync::Arc::new(self.#fname.fork_with_id(new_id)) }
            }
            _ => quote! { #fname: self.#fname.fork_with_id(new_id) },
        }
    });

    quote! {
        pub fn fork(&self) -> Self {
            self.fork_with_id(#crate_name::uuid::Uuid::new_v4())
        }

        #[doc(hidden)]
        pub fn fork_with_id(&self, new_id: #crate_name::uuid::Uuid) -> Self {
            Self {
                __amethystate_instance_id: #crate_name::store::instances::InstanceGuard::new(
                    new_id,
                    ::std::any::type_name::<Self>(),
                ),
                __amethystate_at: self.__amethystate_at.clone(),
                #(#each,)*
            }
        }
    }
}

/// `Debug` for the struct, printing each field that can be printed.
///
/// A field whose value does not implement `Debug` is not a reason for the
/// struct to lose its own: the two traits below pick the real one where there
/// is one and `<opaque>` where there is not, by the trick that an inherent
/// method on `&T` is reached only when the one on `T` does not apply.
pub(crate) fn debug(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    if !schema.mode.watches() {
        return quote! {};
    }

    let name = &schema.name;
    let each = schema.fields.iter().map(|field| {
        let fname = &field.ident;
        let shown = shown_value(crate_name, quote! { &self.#fname });
        quote! { .field(stringify!(#fname), #shown) }
    });

    quote! {
        impl ::std::fmt::Debug for #name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                use #crate_name::observability::{ShownAsOpaque, ShownByDebug};

                f.debug_struct(stringify!(#name))
                    #(#each)*
                    .finish()
            }
        }
    }
}

/// A borrowed value as something printable, whether or not its type is.
///
/// `held` is a reference expression, and what comes back borrows from it - so
/// it has to name something that outlives the printing, not a local made on
/// the way.
///
/// Both traits have to be in scope where this lands: the inherent pick between
/// them is what stands in for a bound nobody can write.
pub(crate) fn shown_value(crate_name: &TokenStream2, held: TokenStream2) -> TokenStream2 {
    quote! {
        (&#crate_name::observability::Shown(#held)).shown()
    }
}
