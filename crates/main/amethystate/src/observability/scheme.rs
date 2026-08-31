use crate::migration::fields::FieldDescriptor;
use amethystate_core::path::StorePath;

pub struct SchemaEntry {
    /// Where the struct's fields live. `None` for a struct that has no place
    /// of its own - one built under a namespace given at runtime.
    pub prefix: Option<StorePath>,
    pub struct_name: &'static str,
    pub version: u32,
    pub schema_hash: u32,
    pub fields: &'static [FieldDescriptor],
}

inventory::collect!(SchemaEntry);
