//! What this binary declares, collected at link time.
//!
//! One entry per `#[amethystate]` struct, submitted by the macro wherever the
//! struct is written and gathered by [`inventory`](https://docs.rs/inventory)
//! without anything registering it by hand.
//!
//! Not reporting. The migration engine walks these to know what shape the code
//! says it has, [`MigrationSet`](crate::migration::set::MigrationSet) falls
//! back to them for a prefix nobody handed it steps for, and
//! [`Kv`](crate::store::Kv) asks them what a namespace may not overwrite - so
//! a store opening at all depends on this being right, which is a different
//! job from showing a person what a field holds.
//!
//! What it does *not* say is what actually opened. A struct compiled in and
//! never constructed has claimed nothing, and an entry here is not evidence
//! that it did.

use crate::migration::fields::FieldDescriptor;
use amethystate_core::path::StorePath;
use std::sync::OnceLock;

/// One declaration, as the macro submits it.
///
/// `doc(hidden)`: the field layout is the agreement between the macro and this
/// crate, and the two travel together. It is public because an expansion lands
/// in the caller's crate and has to name it - reading it is
/// [`declarations`]'s job, and building one by hand is nobody's.
#[doc(hidden)]
pub struct SchemaEntry {
    /// Where the struct's fields live.
    ///
    /// Every entry has one, because a struct that declares no prefix is a
    /// component: it is built inside a struct that has one, its places are
    /// reached through the field that holds it, and everything here walks into
    /// a holder's fields already. So there is no declaration in this list
    /// standing at no place, and nothing reading it has to ask whether there
    /// is.
    pub prefix: StorePath,

    /// Which line of declarations at `prefix` this is a version of, `None` for
    /// the unnamed one. See [`Lineage`].
    pub id: Option<&'static str>,

    pub struct_name: &'static str,
    pub version: u32,
    pub fields: &'static [FieldDescriptor],
}

inventory::collect!(SchemaEntry);

/// Every declaration this binary carries.
///
/// The one reader of the linker's list. It is fixed for the life of the
/// process, so it is walked on the first ask and handed out as a slice
/// afterwards - and the parts of the system that want to know what is declared
/// ask for the declarations rather than for the mechanism that collected them.
pub fn declarations() -> &'static [&'static SchemaEntry] {
    static COMPILED: OnceLock<Vec<&'static SchemaEntry>> = OnceLock::new();

    COMPILED.get_or_init(|| inventory::iter::<SchemaEntry>.into_iter().collect())
}

/// The declarations standing at `prefix`.
///
/// More than one may: a prefix is not a place, and two structs are allowed to
/// share one as long as the places they own stay apart.
pub fn declarations_at(prefix: &StorePath) -> impl Iterator<Item = &'static SchemaEntry> + '_ {
    declarations()
        .iter()
        .copied()
        .filter(move |entry| entry.prefix == *prefix)
}

/// Every version of one line compiled into this binary.
pub fn declarations_of(lineage: &Lineage) -> impl Iterator<Item = &'static SchemaEntry> + '_ {
    declarations_at(&lineage.prefix).filter(move |entry| lineage.holds(entry))
}

/// The declarations at `prefix` that say what the code holds there *now*.
///
/// Not all of them. The earlier versions of a struct are compiled in too - that
/// is what a migration step takes for an argument - and they are not a second
/// opinion about the store's shape, they are its history. One supersedes another
/// when both are of one [`Lineage`] and it stands at a higher version.
///
/// What is left is one declaration per line: the structs that share a prefix
/// under different `id`s are all here, and the v1 beside its own v2 is not.
pub fn current_at(prefix: &StorePath) -> Vec<&'static SchemaEntry> {
    current().filter(|entry| entry.prefix == *prefix).collect()
}

/// The declarations that say what the code holds now, at every prefix.
///
/// [`current_at`] over the whole binary, for a reader that lays out every
/// declared place at once rather than comparing one prefix against its record.
pub fn current() -> impl Iterator<Item = &'static SchemaEntry> {
    declarations()
        .iter()
        .copied()
        .filter(|one| !superseded(one))
}

/// Two declarations of one line at one version that say different things, if
/// this binary carries any.
///
/// A line's versions are told apart by their number and nothing else, so two
/// different ones at one number leave every reader choosing between them by the
/// order the linker happened to hand them over. Two that declare the same places
/// the same way are one declaration compiled in twice - a struct copied into the
/// module that keeps the old versions - and say nothing twice.
pub fn declared_twice() -> Option<(&'static SchemaEntry, &'static SchemaEntry)> {
    let all = declarations();

    for (index, one) in all.iter().enumerate() {
        for other in &all[index + 1..] {
            if one.prefix == other.prefix
                && one.id == other.id
                && one.version == other.version
                && !same_shape(one, other)
            {
                return Some((one, other));
            }
        }
    }

    None
}

/// Two lines at one prefix that own a place in common, if this binary carries
/// any - and the place, named from the root.
///
/// A prefix is shared by places, not by lines. Two lines standing at one may
/// both be built, and a place both of them own - or one owns inside the other's
/// - would be stored in whichever shape the reader met first.
pub fn claimed_twice() -> Option<(&'static SchemaEntry, &'static SchemaEntry, StorePath)> {
    let standing: Vec<&'static SchemaEntry> = current().collect();

    for (index, one) in standing.iter().enumerate() {
        let mine = crate::store::moved::owned(one.fields);

        for other in &standing[index + 1..] {
            if other.prefix != one.prefix || other.id == one.id {
                continue;
            }

            let theirs = crate::store::moved::owned(other.fields);
            let shared = mine.iter().find(|place| {
                theirs
                    .iter()
                    .any(|their| their.at.starts_with(&place.at) || place.at.starts_with(&their.at))
            });

            if let Some(place) = shared {
                return Some((one, other, one.prefix.join(&place.at)));
            }
        }
    }

    None
}

fn same_shape(one: &SchemaEntry, other: &SchemaEntry) -> bool {
    use crate::store::meta::StoredFieldEntry;

    one.fields
        .iter()
        .map(StoredFieldEntry::from)
        .eq(other.fields.iter().map(StoredFieldEntry::from))
}

fn superseded(one: &SchemaEntry) -> bool {
    declarations_at(&one.prefix).any(|other| other.id == one.id && other.version > one.version)
}

/// One line of declarations: the versions of one struct at one prefix.
///
/// A prefix does not make one by itself, because two structs may share it and
/// each goes through versions of its own. What tells them apart is the `id`
/// each was declared with, and the declarations written without one are the
/// prefix's unnamed line. Nothing about the places a declaration owns enters
/// into it: a version may move every one of them and still be the next version
/// of the same line.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Lineage {
    pub prefix: StorePath,
    pub id: Option<String>,
}

impl Lineage {
    /// The unnamed line at `prefix`.
    pub fn unnamed(prefix: StorePath) -> Self {
        Self { prefix, id: None }
    }

    /// The line declared with `id` at `prefix`.
    pub fn named(prefix: StorePath, id: impl Into<String>) -> Self {
        Self {
            prefix,
            id: Some(id.into()),
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Whether `entry` is one of this line's versions.
    pub fn holds(&self, entry: &SchemaEntry) -> bool {
        entry.prefix == self.prefix && entry.id == self.id()
    }
}

impl From<StorePath> for Lineage {
    fn from(prefix: StorePath) -> Self {
        Self::unnamed(prefix)
    }
}

impl std::fmt::Display for Lineage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.id {
            None => write!(f, "{}", self.prefix),
            Some(id) => write!(f, "{} (`{id}`)", self.prefix),
        }
    }
}

impl SchemaEntry {
    /// The line this declaration is a version of.
    pub fn lineage(&self) -> Lineage {
        Lineage {
            prefix: self.prefix.clone(),
            id: self.id.map(str::to_string),
        }
    }
}
