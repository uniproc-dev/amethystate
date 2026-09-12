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
