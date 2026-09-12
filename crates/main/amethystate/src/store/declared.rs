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

            for entry in crate::schema::declarations() {
                from_fields(&entry.prefix, entry.fields, &mut places);
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

/// How a path meets what a schema declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collision {
    /// The path is a declared one, or lies inside one - a field owns whatever
    /// is under it, since that is the inside of its value, and a map owns its
    /// entries.
    Owned(StorePath),

    /// A declared path lies under this one, so a value here, or a map, would
    /// take the level those paths live on.
    Holds(StorePath),
}

/// How `path` meets every declared schema, if it meets any, and which struct
/// it met.
///
/// Reads the inventory rather than a [`Declared`], because this is asked before
/// anything is built and of what this binary says rather than what a store
/// recorded.
pub fn schema_collision(path: &StorePath) -> Option<(Collision, &'static str)> {
    for entry in crate::schema::declarations() {
        let prefix = &entry.prefix;
        if !path.starts_with(prefix) && !prefix.starts_with(path) {
            continue;
        }

        if let Some(found) = collision(prefix, entry.fields, path) {
            return Some((found, entry.struct_name));
        }
    }

    None
}

/// How `path` meets the paths one schema declared, if it meets them at all.
///
/// A node holds nothing itself and is only the way to the paths below it, so
/// `app.panel` meets a schema through its children, as a [`Collision::Holds`]
/// naming one of them - reached at the level [`FieldDescriptor::below`] puts
/// them, which is this one where the node is flattened.
fn collision(at: &StorePath, fields: &[FieldDescriptor], path: &StorePath) -> Option<Collision> {
    for field in fields {
        match field.owns(at) {
            Some(owned) => {
                if owned.starts_with(path) && owned != *path {
                    return Some(Collision::Holds(owned));
                }
                if path.starts_with(&owned) {
                    return Some(Collision::Owned(owned));
                }
            }
            None => {
                let below = field.below(at);

                if let Some(found) = collision(&below, field.children, path) {
                    return Some(found);
                }
            }
        }
    }

    None
}

/// Every path at or under `at` that a construction marks as seeded.
///
/// A struct marks its own prefix, a nested node marks the path it was built at,
/// and a map marks its own path - so the set is the prefix, every [`Role::Node`]
/// under it, and every [`Role::Map`].
pub fn seeded_namespaces_under(at: &StorePath) -> Vec<StorePath> {
    let mut found = Vec::new();

    for entry in crate::schema::declarations() {
        if !entry.prefix.starts_with(at) {
            continue;
        }

        found.push(entry.prefix.clone());
        collect_seeded(&entry.prefix, entry.fields, &mut found);
    }

    found
}

/// A node is the way to the places under it, and a flattened one lends them no
/// segment - so a map beneath it left its marker at this level rather than one
/// below, and asking for the joined name would clear a marker nothing wrote.
fn collect_seeded(at: &StorePath, fields: &[FieldDescriptor], found: &mut Vec<StorePath>) {
    for field in fields {
        match field.role {
            Role::Node => {
                let below = field.below(at);
                collect_seeded(&below, field.children, found);

                if below != *at {
                    found.push(below);
                }
            }
            Role::Map => found.push(at.join(&field.name.path())),
            Role::Field => {}
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn declaring(places: &[(&[&str], Role)]) -> Declared {
        Declared {
            places: places
                .iter()
                .map(|(at, role)| (StorePath::from_segments(*at), *role))
                .collect(),
        }
    }

    fn at(levels: &[&str]) -> StorePath {
        StorePath::from_segments(levels)
    }

    #[test]
    fn a_maps_own_path_is_a_level_and_what_is_under_it_is_a_value() {
        let declared = declaring(&[(&["ui", "widths"], Role::Map)]);

        assert_eq!(declared.holds(&at(&["ui", "widths"])), Holds::Level);
        assert_eq!(declared.holds(&at(&["ui", "widths", "cpu"])), Holds::Value);
        assert_eq!(
            declared.holds(&at(&["ui", "widths", "cpu", "deeper"])),
            Holds::Value
        );
    }

    #[test]
    fn a_field_is_a_value_at_its_own_path() {
        let declared = declaring(&[(&["ui", "theme"], Role::Field)]);

        assert_eq!(declared.holds(&at(&["ui", "theme"])), Holds::Value);
        assert_eq!(declared.holds(&at(&["ui", "theme", "inner"])), Holds::Value);
        assert_eq!(declared.holds(&at(&["ui"])), Holds::Level);
    }

    #[test]
    fn a_level_on_the_way_to_a_declaration_is_a_level() {
        let declared = declaring(&[(&["ui", "widths"], Role::Map)]);

        assert_eq!(declared.holds(&at(&["ui"])), Holds::Level);
        assert_eq!(declared.holds(&StorePath::root()), Holds::Level);
        assert_eq!(declared.holds(&at(&["elsewhere"])), Holds::Level);
    }
}
