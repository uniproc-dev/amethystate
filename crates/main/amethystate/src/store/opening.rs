//! Why a declared struct would not open.
//!
//! Everything a constructor can fail with and nothing else, so a `match` over
//! it needs no arm for the failures that cannot happen there - a flush, a scan,
//! a migration. Adding a way for a constructor to fail is then a compile error
//! at every caller that was handling the rest.
//!
//! Ordinary [`std::error::Error`], so a caller who does not want to look can
//! `?` it into `anyhow`, `eyre` or a `Box<dyn Error>` and be done.

use crate::store::ReadValue;
use crate::store::StorageError;
use crate::store::places::Taken;
use amethystate_core::failure::{Because, spelled};
use amethystate_core::path::{StorePath, StorePathError};
use error_stack::Report;
use std::fmt;
use std::sync::Arc;

/// What stopped a struct from being built.
pub enum OpenStruct {
    /// A declared check read the stored value and turned it down, in the
    /// check's own words.
    ///
    /// The value decoded perfectly well: what it failed is the application's
    /// own rule. Reached only where the field or its struct said
    /// [`OnUnreadable::Refuse`](crate::store::OnUnreadable::Refuse) - the other
    /// policies keep the default and record a
    /// [`Disagreement`](crate::observability::Disagreement) instead.
    Refused { at: StorePath, said: Arc<str> },

    /// What is stored will not read back as the field's declared type.
    ///
    /// `why` is kept whole because what the codec choked on - the type asked
    /// for, the bytes it found, how many of them - is attached to it.
    WillNotRead { at: StorePath, why: Because },

    /// Another struct already owns that place, so this one would write over it.
    ///
    /// [`Taken`] names both sides at both paths: a collision is only
    /// diagnosable with all four, and neither declaration mentions the other.
    /// Boxed because it is four times the size of every other variant and this
    /// one is the rarest.
    Taken(Box<Taken>),

    /// The levels handed in do not make a path. Only reachable through
    /// [`field_with_path`](crate::store::field_with_path) and its kin, which
    /// take a caller's own levels; a struct written with `#[amethystate]`
    /// carries a path the macro checked while it compiled.
    NotAPath(StorePathError),

    /// The disk, in every sense: the file, the engine, the codec.
    ///
    /// Carries the report whole, so the facts attached along the way - the
    /// key, the table, how many bytes - are still there for whoever wants
    /// them.
    Store(Because),
}

impl OpenStruct {
    /// The whole report as a string, facts and all.
    pub fn explain(&self) -> String {
        match self {
            Self::Store(why) | Self::WillNotRead { why, .. } => why.explain(),
            other => other.to_string(),
        }
    }
}

impl fmt::Display for OpenStruct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { at, said } => {
                write!(f, "a declared check refused what is stored at {at}: {said}")
            }
            Self::WillNotRead { at, .. } => {
                write!(f, "what is stored at {at} will not read back")
            }
            Self::Taken(taken) => write!(f, "{taken}"),
            Self::NotAPath(why) => write!(f, "the field was given no path to sit at: {why}"),
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl fmt::Debug for OpenStruct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        spelled(self, f)
    }
}

impl std::error::Error for OpenStruct {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Store(why) | Self::WillNotRead { why, .. } => {
                why.caused().map(|under| under as &dyn std::error::Error)
            }
            Self::Refused { .. } | Self::Taken(_) => None,
        }
    }
}

impl From<Box<Taken>> for OpenStruct {
    fn from(taken: Box<Taken>) -> Self {
        Self::Taken(taken)
    }
}

impl From<StorePathError> for OpenStruct {
    fn from(why: StorePathError) -> Self {
        Self::NotAPath(why)
    }
}

impl From<Report<StorageError>> for OpenStruct {
    fn from(why: Report<StorageError>) -> Self {
        Self::Store(why.into())
    }
}

/// A read that would not answer while the struct was being built, which is
/// most of what building one does.
impl From<ReadValue> for OpenStruct {
    fn from(why: ReadValue) -> Self {
        match why {
            ReadValue::NotAPath(why) => Self::NotAPath(why),
            ReadValue::WillNotRead { at, why } => Self::WillNotRead { at, why },
            ReadValue::Store(why) => Self::Store(why),
            ReadValue::Closed { at } => Self::Store(
                Report::new(StorageError::Closed)
                    .attach(amethystate_core::facts::Key(at))
                    .into(),
            ),
        }
    }
}

/// A write that would not land while the struct was being built - seeding a
/// map's declared defaults is the one that reaches here.
impl From<amethystate_core::primitives::error::WriteValue> for OpenStruct {
    fn from(why: amethystate_core::primitives::error::WriteValue) -> Self {
        Self::Store(Report::<StorageError>::from(why).into())
    }
}

/// Back into a report, for the plumbing under the boundary.
impl From<OpenStruct> for Report<StorageError> {
    fn from(why: OpenStruct) -> Self {
        match why {
            OpenStruct::Store(report) | OpenStruct::WillNotRead { why: report, .. } => {
                report.into_report()
            }
            OpenStruct::NotAPath(why) => Report::new(why).change_context(StorageError::Path),
            OpenStruct::Refused { at, said } => Report::new(StorageError::Read)
                .attach(amethystate_core::facts::Key(at))
                .attach(amethystate_core::facts::Refused(said.to_string())),
            OpenStruct::Taken(taken) => crate::store::places::refused(&taken),
        }
    }
}

/// Why the store itself would not open.
pub enum OpenStore {
    /// The file or the directory it sits in cannot be used: it is missing, it
    /// is not writable, or something else holds it.
    WouldNotOpen { why: Because },

    /// The store opened, and bringing what was stored up to the declared
    /// schema did not finish.
    ///
    /// Reachable through [`StoreBuilder::build`](crate::StoreBuilder::build)
    /// because a `#[migrate]` step declared by hand still runs there.
    Migrating { why: Because },

    /// The disk, in every sense.
    Store(Because),
}

/// What opening does when the store's own files will not read.
///
/// A file that is not a database, a document that will not parse, bytes some
/// other program wrote there. Not about a directory that cannot be created or
/// a file something else holds: those are refused whatever this says, because
/// starting fresh would neither help nor be able to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WillNotOpen {
    /// The open fails and the files are left exactly as they are, for a person
    /// to look at.
    #[default]
    Refuse,

    /// The files are taken away and an empty store is opened in their place.
    ///
    /// For a store whose contents can be rebuilt - a cache, an index, anything
    /// derived - where a start is worth more than what was in it. Everything
    /// [`StoreLayout::names`](crate::store::StoreLayout::names) names goes, the
    /// rewrite copies included, since a copy of what would not read is not a
    /// recovery. It is said at `warn` before anything is removed.
    ///
    /// Nothing here is undone afterwards: what is gone is gone, and an open
    /// that fails a second time fails with what it said the second time.
    StartFresh,
}

/// What a save does when the file it is about to replace will not read.
///
/// A text store's file is meant to be edited, so it can be left half-typed -
/// and a save that meets one cannot lay its own paths over a document it cannot
/// parse. The three answers below are what a store can do instead, and which is
/// right depends on whose the file is: a settings file a person keeps in their
/// editor, or one only the application was ever going to touch.
///
/// The flat engines never ask: their file is theirs and nobody else writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhenItWillNotRead {
    /// Leave the file alone and let the save come round again, for as long as
    /// this window; once it runs out, set the file aside and write.
    ///
    /// The default, because a file that will not read is usually an editor
    /// mid-keystroke and is a document again a moment later. Nothing is decided
    /// while it might still fix itself: the save is refused, the debouncer
    /// tries again at its retry interval, and what the store holds waits in
    /// memory. Only a file that stays broken for the whole window is treated as
    /// broken rather than busy.
    ///
    /// The window is measured from the first save that met it, not from each
    /// attempt.
    TryAgainFor(std::time::Duration),

    /// The unreadable file is moved aside, under a name that says what it is,
    /// and the save goes ahead at once.
    ///
    /// Nothing is lost and the application keeps running: what a person typed
    /// is still on disk under `<name>.unreadable`. A second one replaces the
    /// first, since two copies of a file that will not read are worth no more
    /// than one.
    SetAside,

    /// The save is refused and the file is left exactly as it is.
    ///
    /// For a file whose contents are somebody's work rather than the
    /// application's: nothing on disk moves until it parses again, and what the
    /// store holds stays in memory, reported through the usual flush failure.
    /// The cost is that writes pile up unsaved for as long as the file stays
    /// broken.
    Refuse,

    /// The save writes its document whole and what was in the file is gone,
    /// with a line in the log and nothing else.
    ///
    /// For a file the application owns outright, where an unreadable one is a
    /// fault to be flattened rather than somebody's half-finished edit.
    Overwrite,
}

impl Default for WhenItWillNotRead {
    /// Long enough for an editor to finish writing and short enough that a file
    /// somebody actually broke is not held against the store all day.
    fn default() -> Self {
        Self::TryAgainFor(std::time::Duration::from_secs(5))
    }
}

impl OpenStore {
    /// What the store said, told apart where a caller would act on it
    /// differently.
    pub fn from_store(why: Report<StorageError>) -> Self {
        match *why.current_context() {
            StorageError::Open => Self::WouldNotOpen { why: why.into() },
            StorageError::Migrate => Self::Migrating { why: why.into() },
            _ => Self::Store(why.into()),
        }
    }

    /// The whole report as a string, facts and all.
    pub fn explain(&self) -> String {
        let (Self::WouldNotOpen { why } | Self::Migrating { why } | Self::Store(why)) = self;
        why.explain()
    }
}

impl fmt::Display for OpenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WouldNotOpen { .. } => f.write_str("the store would not open"),
            Self::Migrating { .. } => f.write_str(
                "the store opened, and the data would not come up to the declared schema",
            ),
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl fmt::Debug for OpenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        spelled(self, f)
    }
}

impl std::error::Error for OpenStore {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let (Self::WouldNotOpen { why } | Self::Migrating { why } | Self::Store(why)) = self;
        why.caused().map(|under| under as &dyn std::error::Error)
    }
}

impl From<Report<StorageError>> for OpenStore {
    fn from(why: Report<StorageError>) -> Self {
        Self::from_store(why)
    }
}

impl From<OpenStore> for Report<StorageError> {
    fn from(why: OpenStore) -> Self {
        let (OpenStore::WouldNotOpen { why }
        | OpenStore::Migrating { why }
        | OpenStore::Store(why)) = why;
        why.into_report()
    }
}
