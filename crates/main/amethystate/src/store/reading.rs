//! Why a read would not answer.

use crate::store::StorageError;
use amethystate_core::failure::one_line;
use amethystate_core::path::{StorePath, StorePathError};
use error_stack::Report;
use std::fmt;
use std::sync::Arc;

/// What stopped a value from being read back.
#[derive(Debug)]
pub enum ReadValue {
    /// The levels handed in do not make a path.
    NotAPath(StorePathError),

    /// Something is stored there, and it is not the type that was asked for.
    ///
    /// `why` is kept whole because what the codec choked on - the type asked
    /// for, the bytes it found, how many of them - is attached to it.
    WillNotRead {
        at: StorePath,
        why: Report<StorageError>,
    },

    /// The store has let go of its file, so it answers nothing.
    Closed { at: StorePath },

    /// The disk, in every sense: the file, the engine, the codec.
    Store(Report<StorageError>),
}

impl ReadValue {
    /// What the store said, told apart where a caller would act on it
    /// differently.
    ///
    /// A codec's refusal is a frame *under* the operation's rather than the
    /// outermost one - a read that found the wrong type says `Read` on top and
    /// `Codec` below - so this asks
    /// [`will_not_read`](crate::store::will_not_read), which is the one place
    /// that difference is decided.
    pub fn from_store(at: &StorePath, why: Report<StorageError>) -> Self {
        if *why.current_context() == StorageError::Closed {
            return Self::Closed { at: at.clone() };
        }

        match crate::store::rules::will_not_read(&why) {
            true => Self::WillNotRead {
                at: at.clone(),
                why,
            },
            false => Self::Store(why),
        }
    }
}

impl fmt::Display for ReadValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAPath(why) => write!(f, "the read was given no path to look at: {why}"),
            Self::WillNotRead { at, why } => {
                write!(
                    f,
                    "what is stored at {at} will not read back: {}",
                    one_line(why)
                )
            }
            Self::Closed { at } => write!(f, "the store was closed, so {at} was not read"),
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl std::error::Error for ReadValue {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Store(why) | Self::WillNotRead { why, .. } => Some(why.current_context()),
            Self::Closed { .. } => None,
        }
    }
}

impl From<StorePathError> for ReadValue {
    fn from(why: StorePathError) -> Self {
        Self::NotAPath(why)
    }
}

impl From<Report<StorageError>> for ReadValue {
    fn from(why: Report<StorageError>) -> Self {
        Self::Store(why)
    }
}

impl From<ReadValue> for Report<StorageError> {
    fn from(why: ReadValue) -> Self {
        match why {
            ReadValue::Store(report) | ReadValue::WillNotRead { why: report, .. } => report,
            ReadValue::NotAPath(why) => Report::new(why).change_context(StorageError::Path),
            ReadValue::Closed { at } => {
                Report::new(StorageError::Closed).attach(amethystate_core::facts::Key(at))
            }
        }
    }
}

/// What stopped a listing from coming back.
#[derive(Debug)]
pub enum ScanKeys {
    /// The levels handed in do not make a prefix.
    NotAPath(StorePathError),

    /// A key on disk will not read back as a path.
    ///
    /// Reachable only where something other than this library wrote it, so
    /// `why` carries the key as it sits on disk - which is the only state it
    /// is in when the reason for the failure is that it is not a path.
    KeyWillNotRead {
        under: StorePath,
        why: Report<StorageError>,
    },

    /// The store has let go of its file, so there is nothing to list.
    Closed { under: StorePath },

    /// The disk, in every sense.
    Store(Report<StorageError>),
}

impl ScanKeys {
    /// What the store said, told apart where a caller would act on it
    /// differently.
    pub fn from_store(under: &StorePath, why: Report<StorageError>) -> Self {
        match *why.current_context() {
            StorageError::Path => Self::KeyWillNotRead {
                under: under.clone(),
                why,
            },
            StorageError::Closed => Self::Closed {
                under: under.clone(),
            },
            _ => Self::Store(why),
        }
    }
}

impl fmt::Display for ScanKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAPath(why) => write!(f, "the scan was given no prefix to list: {why}"),
            Self::KeyWillNotRead { under, why } => {
                write!(
                    f,
                    "a key stored under {under} will not read back as a path: {}",
                    one_line(why)
                )
            }
            Self::Closed { under } => {
                write!(f, "the store was closed, so {under} was not listed")
            }
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl std::error::Error for ScanKeys {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Store(why) | Self::KeyWillNotRead { why, .. } => Some(why.current_context()),
            Self::Closed { .. } => None,
        }
    }
}

impl From<StorePathError> for ScanKeys {
    fn from(why: StorePathError) -> Self {
        Self::NotAPath(why)
    }
}

impl From<Report<StorageError>> for ScanKeys {
    fn from(why: Report<StorageError>) -> Self {
        Self::Store(why)
    }
}

impl From<ScanKeys> for Report<StorageError> {
    fn from(why: ScanKeys) -> Self {
        match why {
            ScanKeys::Store(report) | ScanKeys::KeyWillNotRead { why: report, .. } => report,
            ScanKeys::NotAPath(why) => Report::new(why).change_context(StorageError::Path),
            ScanKeys::Closed { under } => {
                Report::new(StorageError::Closed).attach(amethystate_core::facts::Prefix(under))
            }
        }
    }
}

pub type ReadResult<T> = Result<T, ReadValue>;
pub type ScanResult<T> = Result<T, ScanKeys>;

/// What stopped a map from being read back whole.
///
/// Its own set rather than [`OpenStruct`](crate::store::OpenStruct): a map is
/// opened over what is already under it, so it meets two failures nothing else
/// can - a stored key that is not one of its entries, and an entry whose name
/// will not read as its key type.
#[derive(Debug)]
pub enum LoadMap {
    /// The levels handed in do not make a path.
    NotAPath(StorePathError),

    /// Another declaration already owns that place.
    Taken(Box<crate::store::places::Taken>),

    /// A key under the map is not one of its entries.
    ///
    /// A map owns the level below it and nothing further, so a key deeper than
    /// that belongs to whatever claimed that level. `stored` is the key as it
    /// sits on disk, which is the only state it is in when the reason for the
    /// failure is where it sits.
    KeyIsNotAnEntry {
        under: StorePath,
        stored: Arc<str>,
        said: Arc<str>,
    },

    /// An entry's name will not read back as the map's key type.
    KeyWillNotRead {
        under: StorePath,
        entry: Arc<str>,
        wanted: &'static str,
    },

    /// An entry's value will not read back as the map's value type.
    EntryWillNotRead {
        at: StorePath,
        why: Report<StorageError>,
    },

    /// The store has let go of its file.
    Closed { under: StorePath },

    /// The disk, in every sense.
    Store(Report<StorageError>),
}

impl LoadMap {
    /// What the store said, told apart where a caller would act on it
    /// differently.
    pub fn from_store(at: &StorePath, why: Report<StorageError>) -> Self {
        if *why.current_context() == StorageError::Closed {
            return Self::Closed { under: at.clone() };
        }

        match crate::store::rules::will_not_read(&why) {
            true => Self::EntryWillNotRead {
                at: at.clone(),
                why,
            },
            false => Self::Store(why),
        }
    }
}

impl fmt::Display for LoadMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAPath(why) => write!(f, "the map was given no path to sit at: {why}"),
            Self::Taken(taken) => write!(f, "{taken}"),
            Self::KeyIsNotAnEntry {
                under,
                stored,
                said,
            } => write!(f, "`{stored}` is not an entry of {under}: {said}"),
            Self::KeyWillNotRead {
                under,
                entry,
                wanted,
            } => write!(f, "`{entry}` under {under} will not read as a {wanted}"),
            Self::EntryWillNotRead { at, why } => {
                write!(f, "the entry at {at} will not read back: {}", one_line(why))
            }
            Self::Closed { under } => {
                write!(f, "the store was closed, so {under} was not read")
            }
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl std::error::Error for LoadMap {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Store(why) | Self::EntryWillNotRead { why, .. } => Some(why.current_context()),
            Self::Taken(_)
            | Self::KeyIsNotAnEntry { .. }
            | Self::KeyWillNotRead { .. }
            | Self::Closed { .. } => None,
        }
    }
}

impl From<StorePathError> for LoadMap {
    fn from(why: StorePathError) -> Self {
        Self::NotAPath(why)
    }
}

impl From<Report<StorageError>> for LoadMap {
    fn from(why: Report<StorageError>) -> Self {
        Self::Store(why)
    }
}

impl From<Box<crate::store::places::Taken>> for LoadMap {
    fn from(taken: Box<crate::store::places::Taken>) -> Self {
        Self::Taken(taken)
    }
}

/// A write that would not land while the map was being built - seeding its
/// declared defaults is the one that reaches here.
impl From<amethystate_core::primitives::error::WriteValue> for LoadMap {
    fn from(why: amethystate_core::primitives::error::WriteValue) -> Self {
        Self::Store(why.into())
    }
}

impl From<LoadMap> for Report<StorageError> {
    fn from(why: LoadMap) -> Self {
        match why {
            LoadMap::Store(report) | LoadMap::EntryWillNotRead { why: report, .. } => report,
            LoadMap::NotAPath(why) => Report::new(why).change_context(StorageError::Path),
            LoadMap::Taken(taken) => crate::store::places::refused(&taken),
            LoadMap::KeyIsNotAnEntry {
                under,
                stored,
                said,
            } => Report::new(StorageError::Path)
                .attach(amethystate_core::facts::Prefix(under))
                .attach(amethystate_core::facts::RawKey(stored.to_string()))
                .attach(said.to_string()),
            LoadMap::KeyWillNotRead {
                under,
                entry,
                wanted,
            } => Report::new(StorageError::Codec)
                .attach(amethystate_core::facts::Prefix(under))
                .attach(amethystate_core::facts::Entry(entry.to_string()))
                .attach(format!("key type: {wanted}")),
            LoadMap::Closed { under } => {
                Report::new(StorageError::Closed).attach(amethystate_core::facts::Prefix(under))
            }
        }
    }
}

/// A map that would not load stopped a struct from opening.
impl From<LoadMap> for crate::store::OpenStruct {
    fn from(why: LoadMap) -> Self {
        match why {
            LoadMap::NotAPath(why) => Self::NotAPath(why),
            LoadMap::Taken(taken) => Self::Taken(taken),
            LoadMap::EntryWillNotRead { at, why } => Self::WillNotRead { at, why },
            other => Self::Store(other.into()),
        }
    }
}

pub type LoadMapResult<T> = Result<T, LoadMap>;
