use crate::migration::fields::{FieldDescriptor, Role};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct PrefixMeta {
    pub version: u32,
    pub hash: u32,
}

/// What the type said a declared path is, written down.
///
/// Read off the type by [`Probe`](crate::shape::Probe) and carried here so the
/// store holds it too, alongside the code that opened it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StoredShape {
    pub role: Role,

    /// Whether the path may hold nothing and still be a path.
    pub optional: bool,

    /// For a [`Role::Node`], the paths that live under it.
    ///
    /// Left out of the file when empty, which most paths are - a document a
    /// person reads should not carry `"children": []` on every leaf.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<StoredFieldEntry>,
}

impl StoredShape {
    /// One value, always present, holding nothing under it.
    pub const fn field() -> Self {
        Self {
            role: Role::Field,
            optional: false,
            children: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StoredFieldEntry {
    pub name: String,

    /// How the type was spelled, for a person reading the file or the
    /// inspector. A spelling changes when a rename or an alias does while the
    /// type stays what it was, so drift is judged by the hashes and nothing
    /// compares this.
    pub type_name: String,

    /// What the path is, as the type answered when it was written.
    pub shape: StoredShape,
}

impl From<&FieldDescriptor> for StoredFieldEntry {
    fn from(field: &FieldDescriptor) -> Self {
        Self {
            name: field.name.to_string(),
            type_name: field.type_name.to_string(),
            shape: StoredShape {
                role: field.role,
                optional: field.optional,
                children: field.children.iter().map(Self::from).collect(),
            },
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SchemaSnapshot {
    pub version: u32,
    pub struct_name: Option<String>,
    pub schema_hash: u32,
    pub fields: Vec<StoredFieldEntry>,
}
