use crate::migration::fields::{FieldDescriptor, Role};
use crate::store::StorePath;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct PrefixMeta {
    pub version: u32,
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

    /// Whether this node gave the paths under it a segment of its own.
    ///
    /// A node written as `#[serde(flatten)]` did not, so its children sit where
    /// it sits. Recorded rather than folded away, so that a node gaining or
    /// losing its segment reads as that one change and not as every path under
    /// it being replaced.
    ///
    /// Left out of the file when false, which every leaf and most nodes are.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flattened: bool,
}

impl StoredShape {
    /// One value, always present, holding nothing under it.
    pub const fn field() -> Self {
        Self {
            role: Role::Field,
            optional: false,
            children: Vec::new(),
            flattened: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StoredFieldEntry {
    /// Where the field sits under its holder, as a path rather than as the
    /// string it was written as.
    ///
    /// A name is not always one level - `path = "ui.theme"` writes one that is
    /// two - so the difference between a level and a name holding a separator
    /// has to survive the round trip, and only the type keeps it. Written by
    /// the macro and so valid when it is written; read back through
    /// [`StorePath`]'s own `Deserialize`, which refuses a nameless level or a
    /// dangling escape at the document rather than downstream.
    pub name: StorePath,

    /// How the type was spelled, for a person reading the file or the
    /// inspector. A spelling changes when a rename or an alias does while the
    /// type stays what it was, so nothing compares this: drift is judged by
    /// where the declared places sit, and a leaf's contents are answered where
    /// they are read.
    pub type_name: String,

    /// What the path is, as the type answered when it was written.
    pub shape: StoredShape,
}

/// The only place a stored entry is made, and the reason its name needs no
/// checking anywhere else: the macro writes `FieldDescriptor::name` and refuses
/// a nameless level or an escape where the path is written, so what arrives
/// here is a path the compiler already agreed to.
impl From<&FieldDescriptor> for StoredFieldEntry {
    fn from(field: &FieldDescriptor) -> Self {
        Self {
            name: field.name.path(),
            type_name: field.type_name.to_string(),
            shape: StoredShape {
                role: field.role,
                optional: field.optional,
                children: field.children.iter().map(Self::from).collect(),
                flattened: field.flattened,
            },
        }
    }
}

/// The places declared at one prefix, written down.
///
/// A bookkeeping record only ever grows: no field is removed, none is retyped
/// or repurposed under its own name, and one added after the first release
/// carries `#[serde(default)]` so an older record still reads. The other
/// direction - a record a newer build wrote, read by an older one - holds
/// because no record refuses a field it does not know, which the
/// `a_record_carries_a_field_this_build_has_no_name_for` test states.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SchemaSnapshot {
    pub version: u32,

    /// A label, for a report and for a person reading the file.
    ///
    /// Nothing in a migration reads it. A name is a `&'static str` from a type
    /// that may not exist in the next build, two builds may spell one name two
    /// ways, and one type may be renamed while its places stay exactly where
    /// they were - so a rename is not a change to the store and must not be
    /// read as one. The places are the identity.
    ///
    /// The third instance of the same rule, after
    /// [`StoredFieldEntry::type_name`] and `Owner::by`. See
    /// `RFC-the-ownership-tree.md`.
    pub struct_name: Option<String>,

    pub fields: Vec<StoredFieldEntry>,
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::migration::AppliedStep;
    use serde_json::{Value, json};

    fn with_a_stranger(mut record: Value) -> Value {
        record["a_field_from_a_later_build"] = json!("whatever it holds");
        record
    }

    #[test]
    fn a_record_carries_a_field_this_build_has_no_name_for() {
        let shape = json!({ "role": "field", "optional": false });
        let entry = json!({ "name": "width", "type_name": "u32", "shape": shape });

        serde_json::from_value::<PrefixMeta>(with_a_stranger(json!({ "version": 3 }))).unwrap();
        serde_json::from_value::<StoredShape>(with_a_stranger(shape.clone())).unwrap();
        serde_json::from_value::<StoredFieldEntry>(with_a_stranger(entry.clone())).unwrap();
        serde_json::from_value::<SchemaSnapshot>(with_a_stranger(json!({
            "version": 1,
            "struct_name": "Ui",
            "fields": [entry],
        })))
        .unwrap();
        serde_json::from_value::<AppliedStep>(with_a_stranger(json!({
            "prefix": "ui",
            "target_version": 2,
            "description": null,
            "applied_at": 0,
        })))
        .unwrap();
    }
}
