//! Why a write through [`Kv`](crate::store::Kv) would not land.
//!
//! Its own set rather than a variant on
//! [`WriteValue`](crate::errors::WriteValue): a raw write is the one that can
//! land on a place a declared struct owns, and a field write is the one that
//! can be turned down by an interceptor or aimed at a cell whose source is
//! gone. Neither should have to read past the other's failures.

use crate::store::StorageError;
use amethystate_core::failure::{Because, spelled};
use amethystate_core::path::{StorePath, StorePathError};
use error_stack::Report;
use std::fmt;

/// What stopped a raw write.
pub enum KvWrite {
    /// The name handed in cannot be a level.
    NotAPath(StorePathError),

    /// A declared struct owns that place, so the write would go under it.
    ///
    /// `declared_at` is the path the schema declared, which is `at` itself or
    /// one it lies inside, and `by` is the struct that declared it. What is
    /// owned is the declared path and whatever lies inside it; the prefix it
    /// sits under stays open.
    Declared {
        at: StorePath,
        declared_at: StorePath,
        by: &'static str,
    },

    /// The path or the value nests deeper than this store reads back.
    ///
    /// `why` is kept whole because the numbers are the diagnosis: which budget
    /// ran out, what it was, and how much the path had already spent.
    TooDeep { at: StorePath, why: Because },

    /// The value will not turn into what the store keeps, with what the codec
    /// said kept whole.
    WillNotEncode { at: StorePath, why: Because },

    /// The store has let go of its file, so nothing lands.
    ///
    /// No report: every [`StorageError::Closed`] is minted where the refusal
    /// is, so nothing has failed underneath it.
    Closed { at: StorePath },

    /// The disk, in every sense: the file, the engine, the codec.
    Store(Because),
}

amethystate_core::what_the_store_said!(KvWrite);

impl fmt::Display for KvWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAPath(why) => write!(f, "the write was given no path to land at: {why}"),
            Self::Declared {
                at,
                declared_at,
                by,
            } if at == declared_at => {
                write!(f, "{at} is declared by {by}")
            }
            Self::Declared {
                at,
                declared_at,
                by,
            } if declared_at.starts_with(at) => {
                write!(f, "{at} holds {declared_at}, which {by} declares")
            }
            Self::Declared {
                at,
                declared_at,
                by,
            } => {
                write!(f, "{at} is inside {declared_at}, which {by} declares")
            }
            Self::TooDeep { at, .. } => write!(f, "{at} is deeper than this store reads back"),
            Self::WillNotEncode { at, .. } => {
                write!(f, "what was written to {at} will not encode")
            }
            Self::Closed { at } => {
                write!(f, "the store was closed, so nothing was written to {at}")
            }
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl fmt::Debug for KvWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        spelled(self, f)
    }
}

impl std::error::Error for KvWrite {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Store(why) | Self::TooDeep { why, .. } | Self::WillNotEncode { why, .. } => {
                why.caused().map(|under| under as &dyn std::error::Error)
            }
            Self::Declared { .. } | Self::Closed { .. } => None,
        }
    }
}

impl From<StorePathError> for KvWrite {
    fn from(why: StorePathError) -> Self {
        Self::NotAPath(why)
    }
}

impl From<Report<StorageError>> for KvWrite {
    fn from(why: Report<StorageError>) -> Self {
        Self::Store(why.into())
    }
}

impl From<KvWrite> for Report<StorageError> {
    fn from(why: KvWrite) -> Self {
        match why {
            KvWrite::Store(report)
            | KvWrite::TooDeep { why: report, .. }
            | KvWrite::WillNotEncode { why: report, .. } => report.into_report(),
            KvWrite::NotAPath(why) => Report::new(why).change_context(StorageError::Path),
            KvWrite::Closed { at } => {
                Report::new(StorageError::Closed).attach(amethystate_core::facts::Key(at))
            }
            KvWrite::Declared {
                at,
                declared_at,
                by,
            } => Report::new(StorageError::Taken)
                .attach(amethystate_core::facts::Key(at))
                .attach(crate::store::places::Owner {
                    path: declared_at,
                    by,
                }),
        }
    }
}

pub type KvResult<T> = Result<T, KvWrite>;

/// Why the buffered writes did not reach disk.
pub enum Flush {
    /// The store has already let go of its file.
    Closed,

    /// Asked for from inside something the store is already doing, where doing
    /// it would mean waiting for the caller to finish.
    Reentrant,

    /// The commit itself failed, with how much was still buffered attached.
    DidNotLand { why: Because },
}

impl Flush {
    /// What the store said, told apart where a caller would act on it
    /// differently.
    pub fn from_store(why: Report<StorageError>) -> Self {
        match *why.current_context() {
            StorageError::Closed => Self::Closed,
            StorageError::Reentrant => Self::Reentrant,
            _ => Self::DidNotLand { why: why.into() },
        }
    }

    /// The whole report as a string, facts and all.
    pub fn explain(&self) -> String {
        match self {
            Self::DidNotLand { why } => why.explain(),
            other => other.to_string(),
        }
    }
}

impl fmt::Display for Flush {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("the store was already closed, so nothing was flushed"),
            Self::Reentrant => {
                f.write_str("a flush was asked for from inside what the store is already doing")
            }
            Self::DidNotLand { .. } => f.write_str("the flush did not land"),
        }
    }
}

impl fmt::Debug for Flush {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        spelled(self, f)
    }
}

impl std::error::Error for Flush {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DidNotLand { why } => why.caused().map(|under| under as &dyn std::error::Error),
            Self::Closed | Self::Reentrant => None,
        }
    }
}

impl From<Report<StorageError>> for Flush {
    fn from(why: Report<StorageError>) -> Self {
        Self::from_store(why)
    }
}

impl From<Flush> for Report<StorageError> {
    fn from(why: Flush) -> Self {
        match why {
            Flush::DidNotLand { why } => why.into_report(),
            Flush::Closed => Report::new(StorageError::Closed),
            Flush::Reentrant => Report::new(StorageError::Reentrant),
        }
    }
}

pub type FlushResult<T> = Result<T, Flush>;
