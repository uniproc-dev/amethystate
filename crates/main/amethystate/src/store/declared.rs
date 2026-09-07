//! What the declarations say about a path, for the two questions a document
//! cannot answer out of its own file.
//!
//! A flat engine has both answers in its keys: a struct written at `a.b` is one
//! key, and `a.b.x` beside it is another. A document writes an object either
//! way, so it asks here instead.
//!
//! [`Declared::covers`] is asked first, and it says where the path is written:
//! a declared place goes in the tree, level by level, and everything else in
//! the plane of whole keys beside it, which the text engines' `layout` module
//! lays out.
//!
//! [`Declared::holds`] is asked of the tree, and it says where a walk of it
//! stops: a [`Role::Field`] holds one value and whatever is under it is that
//! value's own shape, and a [`Role::Map`] holds a level whose entries are one
//! value each.
//!
//! Two sources, both questions. [`Declared::compiled_in`] is what this binary
//! says and is there before the store opens; [`Declared::record`] takes what
//! the store wrote down when a binary carrying those declarations last opened
//! it, which is all a tool with none of its own has to go on.

use crate::migration::fields::{FieldDescriptor, Role};
use crate::schema::SchemaEntry;
use crate::store::meta::StoredFieldEntry;
use amethystate_core::path::StorePath;
use std::sync::OnceLock;

/// What a scan finds at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holds {
    /// One value, taken whole. Nothing under it is a path.
    Value,

    /// A level, whose contents are paths of their own.
    Level,
}

/// The declared places, and what each one holds.
#[derive(Debug, Default, Clone)]
pub struct Declared {
    places: Vec<(StorePath, Role)>,
}

impl Declared {
    /// What this binary declares, gathered once.
    ///
    /// The inventory is fixed for the life of the process, so this is built on
    /// the first ask and handed out by reference afterwards.
    pub fn compiled_in() -> &'static Declared {
        static COMPILED: OnceLock<Declared> = OnceLock::new();

        COMPILED.get_or_init(|| {
            let mut places = Vec::new();

            for entry in inventory::iter::<SchemaEntry> {
                if let Some(prefix) = &entry.prefix {
                    from_fields(prefix, entry.fields, &mut places);
                }
            }

            Declared { places }
        })
    }

    /// Adds what the store recorded under `prefix`.
    ///
    /// Appended after what the binary declares, so [`Declared::holds`] - which
    /// takes the first match - answers from the binary where the two disagree:
    /// it is the code that is about to read the values. [`Declared::covers`]
    /// and [`Declared::owns_level`] ask whether any place matches, so for them
    /// a recorded prefix widens the answer rather than losing to it.
    pub fn record(&mut self, prefix: &StorePath, fields: &[StoredFieldEntry]) {
        from_stored(prefix, fields, &mut self.places);
    }

    /// The declarations that can bear on a scan under `prefix`, which is all a
    /// walk of it has to ask.
    pub fn under(&self, prefix: &StorePath) -> Declared {
        Declared {
            places: self
                .places
                .iter()
                .filter(|(at, _)| at.overlaps(prefix))
                .cloned()
                .collect(),
        }
    }

    /// Whether a declaration puts `path` in the tree.
    ///
    /// This is what divides a document in two. A declared place is written as a
    /// tree, level by level, along with every level on the way to it and every
    /// entry on a map's level - that is what makes a settings file worth opening
    /// in an editor. Everything else is written whole, as one name, because
    /// nothing says where its levels would end and its value would begin.
    ///
    /// A path *inside* a declared value is not in the tree. `name` holding a
    /// `String` says nothing about `name.inner`, which no declaration mentions
    /// and which is a path of its own - the same key a flat engine would hold
    /// beside it, and the same key serde would read out of a bare file.
    pub fn covers(&self, path: &StorePath) -> bool {
        self.places
            .iter()
            .any(|(at, role)| at.starts_with(path) || (role.same(Role::Map) && entry_of(at, path)))
    }

    /// Whether a declaration owns `path` as a level of its own.
    ///
    /// A map is the one place that is a level: its entries are paths, and the
    /// level stands whether or not it holds any. A document that would drop a
    /// level it just emptied has to ask this first - an emptied map that
    /// leaves nothing behind is a map that was never there, and the difference
    /// is whether its defaults come back.
    pub fn owns_level(&self, path: &StorePath) -> bool {
        self.places
            .iter()
            .any(|(at, role)| role.same(Role::Map) && at == path)
    }

    /// What is stored at `path`.
    pub fn holds(&self, path: &StorePath) -> Holds {
        for (at, role) in &self.places {
            let inside = match role {
                Role::Field => path.starts_with(at),
                Role::Map => path.len() > at.len() && path.starts_with(at),
                Role::Node => false,
            };

            if inside {
                return Holds::Value;
            }
        }

        Holds::Level
    }
}

/// Whether `path` is one of the entries a map at `at` owns, which is the level
/// below it and nothing further.
fn entry_of(at: &StorePath, path: &StorePath) -> bool {
    path.len() == at.len() + 1 && path.starts_with(at)
}

fn from_fields(at: &StorePath, fields: &[FieldDescriptor], into: &mut Vec<(StorePath, Role)>) {
    for field in fields {
        match field.owns(at) {
            Some(owned) => into.push((owned, field.role)),
            None => from_fields(&field.below(at), field.children, into),
        }
    }
}

fn from_stored(at: &StorePath, fields: &[StoredFieldEntry], into: &mut Vec<(StorePath, Role)>) {
    for field in fields {
        let shape = &field.shape;

        match shape.role {
            Role::Node => {
                let below = match shape.flattened {
                    true => at.clone(),
                    false => at.join(&field.name),
                };
                from_stored(&below, &shape.children, into);
            }
            role => into.push((at.join(&field.name), role)),
        }
    }
}
