use crate::MigrationContext;
use crate::store::StorageResult;

/// What a declared path is, as far as the store is concerned.
///
/// Not what the value is - that is the value's business and the disk's. This
/// says whether the path holds one value, or is the level a map's entries sit
/// under, or is only a level on the way to other declared paths.
/// Written as a string rather than as a variant, because a `ron::value::Value`
/// has nowhere to put a unit variant and refuses to read one back.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(into = "&'static str", try_from = "String")]
pub enum Role {
    /// One value lives here. Anything under it in a document is the inside of
    /// that value, not a path.
    Field,

    /// A map's entries live one level under here. This path itself holds
    /// nothing.
    Map,

    /// A level on the way to declared paths, holding nothing itself.
    Node,
}

impl Role {
    /// `==` where a `const` needs it, which the derived `PartialEq` cannot
    /// answer.
    pub const fn same(self, other: Self) -> bool {
        self as u8 == other as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Field => "field",
            Role::Map => "map",
            Role::Node => "node",
        }
    }
}

impl From<Role> for &'static str {
    fn from(role: Role) -> Self {
        role.as_str()
    }
}

impl TryFrom<String> for Role {
    type Error = String;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        match name.as_str() {
            "field" => Ok(Role::Field),
            "map" => Ok(Role::Map),
            "node" => Ok(Role::Node),
            other => Err(format!("no such role: {other}")),
        }
    }
}

#[derive(Clone)]
pub struct FieldDescriptor {
    pub name: &'static str,
    pub type_hash: u32,
    pub type_name: &'static str,

    pub role: Role,

    /// Whether the path may hold nothing and still be a path - which is not the
    /// same as the path being absent, and is written differently by every
    /// engine that can write it at all.
    ///
    /// Read from the type by [`Probe`](crate::shape::Probe).
    pub optional: bool,

    /// For a [`Role::Node`], the fields that live under it; empty otherwise.
    ///
    /// See [`FieldDescriptor::leaf`] for the ordinary case.
    ///
    /// A static reference rather than a walk, so the set of declared paths is
    /// known without opening the store. It cannot be cyclic: a construction
    /// cycle is refused at compile time by
    /// [`AmeStateNode::CONSTRUCTION_TERMINATES`](crate::AmeStateNode::CONSTRUCTION_TERMINATES).
    pub children: &'static [FieldDescriptor],
}

impl FieldDescriptor {
    /// A path holding one value, which is what most declared paths are.
    pub const fn leaf(name: &'static str, type_hash: u32, type_name: &'static str) -> Self {
        Self {
            name,
            type_hash,
            type_name,
            role: Role::Field,
            optional: false,
            children: &[],
        }
    }
}

pub trait AmeStateFields: Sized {
    const FIELDS: &'static [FieldDescriptor];
    const VERSION: u32;
    const SCHEMA_HASH: u32;
    const PARENT_PREFIX: &'static str;
    const MIGRATION_DEPS: &'static [&'static str];

    fn load_struct(ctx: &mut MigrationContext) -> StorageResult<Self>;

    fn save_struct(&self, ctx: &mut MigrationContext) -> StorageResult<()>;
}
