use crate::MigrationContext;
use crate::store::{StaticPath, StorePath};

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
    /// Where it is stored under its holder, which is its own name unless
    /// `path` or `rename_all` said otherwise.
    ///
    /// A path rather than a string, because a name is not always one level and
    /// a level is not always one name: `path = "ui.theme"` writes two levels,
    /// and a field whose name holds a separator writes one level that escapes
    /// it. The macro builds both halves and [`StaticPath`] checks they agree
    /// while the code is compiled.
    pub name: StaticPath,

    /// The name in the source, which is what the code calls it. Told apart
    /// from [`name`](FieldDescriptor::name) because a person editing the file
    /// and a person reading the code are looking at different words.
    pub declared: &'static str,

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

    /// Whether this node gives the paths under it a segment of its own.
    ///
    /// A flattened node does not: its children sit at its holder's level, so
    /// anything walking this tree to reach a path has to pass through without
    /// adding [`name`](FieldDescriptor::name). Written as `#[serde(flatten)]`,
    /// and false for everything that is not a [`Role::Node`].
    pub flattened: bool,
}

impl FieldDescriptor {
    /// The place this declaration owns, under `at`, or `None` when it owns
    /// none.
    ///
    /// A node is not a place. Nothing is stored at one,
    /// [`Places::take`](crate::store::places::Places::take) is never called
    /// for one, and a write beside its fields belongs to whoever wrote it - it
    /// is the way to the paths below it and nothing else. A leaf owns its path
    /// and whatever is inside its value; a map owns its path and every entry
    /// under it.
    ///
    /// The one place this question is answered, so that a walk over
    /// declarations cannot answer it differently from the next walk.
    pub fn owns(&self, at: &StorePath) -> Option<StorePath> {
        match self.role {
            Role::Node => None,
            Role::Field | Role::Map => Some(at.join(&self.name.path())),
        }
    }

    /// The level the paths under this declaration sit at.
    ///
    /// Its own, unless it is flattened - then its fields sit where it does,
    /// and it lends them no segment.
    pub fn below(&self, at: &StorePath) -> StorePath {
        match self.flattened {
            true => at.clone(),
            false => at.join(&self.name.path()),
        }
    }

    /// A path holding one value, which is what most declared paths are.
    ///
    /// Both halves of the path are written out, because a `const` cannot build
    /// the levels from the joined form: [`StaticPath`] checks they agree, and a
    /// pair that does not is a compile error.
    pub const fn leaf(
        segments: &'static [&'static str],
        joined: &'static str,
        type_name: &'static str,
    ) -> Self {
        Self {
            name: StaticPath::new(segments, joined),
            declared: joined,
            type_name,
            role: Role::Field,
            optional: false,
            children: &[],
            flattened: false,
        }
    }
}

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }

    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Whether `fields` puts a path called `name` at its own level.
///
/// Reached through flattened nodes, which contribute no segment: a name a
/// flattened grandchild brings up arrives here as if it were written here.
pub const fn brings(fields: &[FieldDescriptor], name: &str) -> bool {
    let mut i = 0;
    while i < fields.len() {
        if fields[i].flattened {
            if brings(fields[i].children, name) {
                return true;
            }
        } else if same(fields[i].name.as_str(), name) {
            return true;
        }
        i += 1;
    }
    false
}

/// Whether two sets of fields, flattened into the same level, would land on a
/// name in common.
pub const fn overlap(a: &[FieldDescriptor], b: &[FieldDescriptor]) -> bool {
    let mut i = 0;
    while i < a.len() {
        if a[i].flattened {
            if overlap(a[i].children, b) {
                return true;
            }
        } else if brings(b, a[i].name.as_str()) {
            return true;
        }
        i += 1;
    }
    false
}

/// Whether a name already spelled at this level is also brought up by `fields`.
pub const fn brings_any(fields: &[FieldDescriptor], names: &[&str]) -> bool {
    let mut i = 0;
    while i < names.len() {
        if brings(fields, names[i]) {
            return true;
        }
        i += 1;
    }
    false
}

pub trait AmeStateFields: Sized {
    const FIELDS: &'static [FieldDescriptor];
    const VERSION: u32;

    /// Where this struct's fields live: the prefix it was declared under, and
    /// the root for a struct declared `as_root`.
    ///
    /// The macro takes the levels apart while it expands, so what is written
    /// here has already been read as a path. `as_root` and `prefix` therefore
    /// arrive as the same kind of thing, and nothing downstream has to know
    /// which one was spelled.
    const PARENT_PREFIX: StaticPath;

    fn load_struct(ctx: &mut MigrationContext) -> crate::migration::StepResult<Self>;

    fn save_struct(&self, ctx: &mut MigrationContext) -> crate::migration::StepResult<()>;
}
