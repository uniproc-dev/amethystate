//! Why a declared struct would not open.
//!
//! Everything a constructor can fail with and nothing else, so a `match` over
//! it needs no arm for the failures that cannot happen there - a flush, a scan,
//! a migration. Adding a way for a constructor to fail is then a compile error
//! at every caller that was handling the rest.
//!
//! Ordinary [`std::error::Error`], so a caller who does not want to look can
//! `?` it into `anyhow`, `eyre` or a `Box<dyn Error>` and be done.

use crate::store::StorageError;
use crate::store::places::Taken;
use amethystate_core::path::{StorePath, StorePathError};
use error_stack::Report;
use std::fmt;
use std::sync::Arc;

/// What stopped a struct from being built.
#[derive(Debug)]
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
    WillNotRead {
        at: StorePath,
        why: Report<StorageError>,
    },

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
    Store(Report<StorageError>),
}

impl fmt::Display for OpenStruct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { at, said } => {
                write!(f, "a declared check refused what is stored at {at}: {said}")
            }
            Self::WillNotRead { at, why } => {
                write!(
                    f,
                    "what is stored at {at} will not read back: {}",
                    amethystate_core::failure::one_line(why)
                )
            }
            Self::Taken(taken) => write!(f, "{taken}"),
            Self::NotAPath(why) => write!(f, "the field was given no path to sit at: {why}"),
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl std::error::Error for OpenStruct {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Store(why) | Self::WillNotRead { why, .. } => Some(why.current_context()),
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
        Self::Store(why)
    }
}

/// A read that would not answer while the struct was being built, which is
/// most of what building one does.
impl From<crate::store::ReadValue> for OpenStruct {
    fn from(why: crate::store::ReadValue) -> Self {
        use crate::store::ReadValue;

        match why {
            ReadValue::NotAPath(why) => Self::NotAPath(why),
            ReadValue::WillNotRead { at, why } => Self::WillNotRead { at, why },
            ReadValue::Store(why) => Self::Store(why),
            ReadValue::Closed { at } => Self::Store(
                Report::new(StorageError::Closed).attach(amethystate_core::facts::Key(at)),
            ),
        }
    }
}

/// A write that would not land while the struct was being built - seeding a
/// map's declared defaults is the one that reaches here.
impl From<amethystate_core::primitives::error::WriteValue> for OpenStruct {
    fn from(why: amethystate_core::primitives::error::WriteValue) -> Self {
        Self::Store(why.into())
    }
}

/// Back into a report, for the plumbing under the boundary.
impl From<OpenStruct> for Report<StorageError> {
    fn from(why: OpenStruct) -> Self {
        match why {
            OpenStruct::Store(report) | OpenStruct::WillNotRead { why: report, .. } => report,
            OpenStruct::NotAPath(why) => Report::new(why).change_context(StorageError::Path),
            OpenStruct::Refused { at, said } => Report::new(StorageError::Read)
                .attach(amethystate_core::facts::Key(at))
                .attach(amethystate_core::facts::Refused(said.to_string())),
            OpenStruct::Taken(taken) => crate::store::places::refused(&taken),
        }
    }
}

/// Why the store itself would not open.
#[derive(Debug)]
pub enum OpenStore {
    /// The file or the directory it sits in cannot be used: it is missing, it
    /// is not writable, or something else holds it.
    WouldNotOpen { why: Report<StorageError> },

    /// The store opened, and bringing what was stored up to the declared
    /// schema did not finish.
    ///
    /// Reachable through [`StoreBuilder::build`](crate::StoreBuilder::build)
    /// because a `#[migrate]` step declared by hand still runs there.
    Migrating { why: Report<StorageError> },

    /// The disk, in every sense.
    Store(Report<StorageError>),
}

impl OpenStore {
    /// What the store said, told apart where a caller would act on it
    /// differently.
    pub fn from_store(why: Report<StorageError>) -> Self {
        match *why.current_context() {
            StorageError::Open => Self::WouldNotOpen { why },
            StorageError::Migrate => Self::Migrating { why },
            _ => Self::Store(why),
        }
    }
}

impl fmt::Display for OpenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let said = |why| amethystate_core::failure::one_line(why);

        match self {
            Self::WouldNotOpen { why } => write!(f, "the store would not open: {}", said(why)),
            Self::Migrating { why } => write!(
                f,
                "the store opened, and the data would not come up to the declared schema: {}",
                said(why)
            ),
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl std::error::Error for OpenStore {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        let (Self::WouldNotOpen { why } | Self::Migrating { why } | Self::Store(why)) = self;
        Some(why.current_context())
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
        why
    }
}
