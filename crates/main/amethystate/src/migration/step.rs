//! Why a migration step would not run.

use crate::store::{IntoStorageReport, StorageError};
use amethystate_core::failure::one_line;
use amethystate_core::path::{StorePath, StorePathError};
use error_stack::Report;
use std::fmt;
use std::sync::Arc;

/// What stopped a step.
///
/// A step is written by whoever uses the library and reads and writes through
/// [`MigrationContext`](crate::MigrationContext); this is what that context
/// hands back, and what the step hands on. The distinction it exists to make
/// is between *this record is not what I expected* - which a step can often
/// skip past - and *the disk is broken*, which it cannot.
#[derive(Debug)]
pub enum RunStep {
    /// The step asked for a value the application never provided.
    ///
    /// A startup mistake rather than a data one:
    /// [`StoreBuilder::provide`](crate::StoreBuilder::provide) hands a value to
    /// every step, and `on_offer` lists what was handed instead.
    NothingProvided {
        under: Arc<str>,
        wanted: &'static str,
        on_offer: Arc<str>,
    },

    /// What is stored will not read back as the shape the step asked for.
    WillNotRead {
        under: Arc<str>,
        entry: Arc<str>,
        wanted: &'static str,
        why: Report<StorageError>,
    },

    /// What the step produced will not encode.
    WillNotEncode {
        under: Arc<str>,
        entry: Arc<str>,
        wanted: &'static str,
        why: Report<StorageError>,
    },

    /// The key the step named cannot be a level.
    NotAPath(StorePathError),

    /// The step turned the data down itself.
    ///
    /// Its own variant so the engine, a log line and a test can tell a step
    /// that decided from a disk that broke under one.
    Refused(crate::MigrationError),

    /// The disk, in every sense.
    Store(Report<StorageError>),
}

impl RunStep {
    /// What the store said, told apart where a step would act on it
    /// differently.
    pub fn from_store(under: &StorePath, entry: &str, why: Report<StorageError>) -> Self {
        match crate::store::will_not_read(&why) {
            true => Self::WillNotRead {
                under: Arc::from(under.to_string()),
                entry: Arc::from(entry),
                wanted: "",
                why,
            },
            false => Self::Store(why),
        }
    }

    /// The same, naming the type the step asked the value to be.
    pub fn reading<T>(under: &StorePath, entry: &str, why: Report<StorageError>) -> Self {
        match Self::from_store(under, entry, why) {
            Self::WillNotRead {
                under, entry, why, ..
            } => Self::WillNotRead {
                under,
                entry,
                wanted: std::any::type_name::<T>(),
                why,
            },
            other => other,
        }
    }

    /// The twin for a value on its way out.
    pub fn writing<T>(under: &StorePath, entry: &str, why: Report<StorageError>) -> Self {
        match crate::store::will_not_read(&why) {
            true => Self::WillNotEncode {
                under: Arc::from(under.to_string()),
                entry: Arc::from(entry),
                wanted: std::any::type_name::<T>(),
                why,
            },
            false => Self::Store(why),
        }
    }
}

impl fmt::Display for RunStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingProvided {
                under,
                wanted,
                on_offer,
            } => write!(
                f,
                "the step under `{under}` asked for a {wanted} and nothing provided one; {on_offer}"
            ),
            Self::WillNotRead {
                under,
                entry,
                wanted,
                why,
            } => write!(
                f,
                "`{entry}` under `{under}` will not read as a {wanted}: {}",
                one_line(why)
            ),
            Self::WillNotEncode {
                under,
                entry,
                wanted,
                why,
            } => write!(
                f,
                "the {wanted} the step wrote to `{entry}` under `{under}` will not encode: {}",
                one_line(why)
            ),
            Self::NotAPath(why) => write!(f, "the step named no path to work at: {why}"),
            Self::Refused(why) => write!(f, "the step turned the data down: {why}"),
            Self::Store(why) => write!(f, "{}", one_line(why)),
        }
    }
}

impl std::error::Error for RunStep {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Refused(why) => Some(why),
            Self::Store(why) | Self::WillNotRead { why, .. } | Self::WillNotEncode { why, .. } => {
                Some(why.current_context())
            }
            Self::NothingProvided { .. } => None,
        }
    }
}

impl From<StorePathError> for RunStep {
    fn from(why: StorePathError) -> Self {
        Self::NotAPath(why)
    }
}

impl From<Report<StorageError>> for RunStep {
    fn from(why: Report<StorageError>) -> Self {
        Self::Store(why)
    }
}

impl From<crate::MigrationError> for RunStep {
    fn from(why: crate::MigrationError) -> Self {
        Self::Refused(why)
    }
}

impl From<RunStep> for Report<StorageError> {
    fn from(why: RunStep) -> Self {
        match why {
            RunStep::Store(report)
            | RunStep::WillNotRead { why: report, .. }
            | RunStep::WillNotEncode { why: report, .. } => report,
            RunStep::NotAPath(why) => Report::new(why).change_context(StorageError::Path),
            RunStep::Refused(why) => why.into_report(),
            RunStep::NothingProvided {
                under,
                wanted,
                on_offer,
            } => Report::new(StorageError::Migrate)
                .attach(crate::store::facts::Migrating(under.to_string()))
                .attach(format!("no value provided for {wanted}"))
                .attach(on_offer.to_string())
                .attach("StoreBuilder::provide hands a value to every migration step"),
        }
    }
}

pub type StepResult<T> = Result<T, RunStep>;
