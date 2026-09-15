//! Everything a declaration becomes when it runs in this process, against a
//! store this process holds.
//!
//! Each part decides for itself whether the declaration asked for it, and
//! hands back nothing when it did not. So the assembly names the parts and
//! nothing else: no part is written twice for the modes that share it.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;

use super::{accessors, data, export, introspect, policy, reactive};
use crate::amethystate::model::Schema;

pub(crate) fn generate(crate_name: &TokenStream2, schema: &Schema) -> TokenStream2 {
    let declaration = reactive::declaration(crate_name, schema);
    let debug = reactive::debug(crate_name, schema);
    let inherent = reactive::inherent(crate_name, schema);

    let scope = accessors::scope(crate_name, schema);
    let node = accessors::node_impl(crate_name, schema);
    let slice = accessors::slice_impl(crate_name, schema);
    let global_new = accessors::global_new(crate_name, schema);

    let data = data::data_impl(crate_name, schema);
    let declared_policy = policy::declared(crate_name, schema);
    let exported = export::entries(crate_name, schema);
    let inspecting = introspect::inspect(crate_name, schema);

    quote! {
        #declaration
        #debug
        #scope
        #inherent
        #global_new
        #node
        #data
        #exported
        #slice
        #declared_policy
        #inspecting
    }
}
