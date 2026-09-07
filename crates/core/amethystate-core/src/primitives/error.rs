use crate::failure::{StorageError, one_line};
use crate::path::{StorePath, StorePathError};
use error_stack::Report;
use std::fmt;
use std::sync::Arc;

/// Everything a write through a reactive primitive can fail with, and nothing
/// else.
///
/// Fields, cells and maps fail in the same ways, so they share one set; the
/// per-primitive names below are aliases kept for readability at call sites.
///
/// Ordinary [`std::error::Error`], so a caller who does not want to look can
/// `?` it into `anyhow`, `eyre` or a `Box<dyn Error>` and be done. A caller who
/// does gets the place and the sentence in the variant rather than out of a
/// bag of attachments.
#[derive(Debug)]
pub enum WriteValue {
    /// An interceptor turned the change down, in its own words.
    Intercepted { at: StorePath, said: Arc<str> },

    /// Nothing is stored where the write was aimed, and this write only
    /// changes what is already there.
    Absent { at: StorePath },

    /// The name handed in cannot be a level, so nothing can sit under it.
    NotAPath(StorePathError),

    /// The path or the value nests deeper than this store reads back.
    ///
    /// `why` is kept whole because the numbers are the diagnosis: which budget
    /// ran out, what it was, and how much the path had already spent are
    /// attached to it.
    TooDeep {
        at: StorePath,
        why: Report<StorageError>,
    },

    /// The value will not turn into what the store keeps, with what the codec
    /// said kept whole.
    WillNotEncode {
        at: StorePath,
        why: Report<StorageError>,
    },

    /// The store has let go of its file, so nothing lands.
    Closed { at: StorePath },

    /// The field or map this cell views has been dropped. `into_cell` is the
    /// form that keeps it alive.
    SourceGone,

    /// The disk, in every sense: the file, the engine, the codec.
    ///
    /// Carries the report whole, so the facts attached along the way - the
    /// key, the table, how many bytes - are still there for whoever wants
    /// them.
    Store(Report<StorageError>),
}

impl WriteValue {
    /// What the store said, told apart where a caller would act on it
    /// differently.
    ///
    /// Only the outermost context is read, and it becomes a type rather than
    /// something to downcast: depth, codec and closed each get a variant of
    /// their own, and anything else travels whole in [`WriteValue::Store`].
    pub fn from_store(at: &StorePath, why: Report<StorageError>) -> Self {
        match *why.current_context() {
            StorageError::Depth => Self::TooDeep {
                at: at.clone(),
                why,
            },
            StorageError::Codec => Self::WillNotEncode {
                at: at.clone(),
                why,
            },
            StorageError::Closed => Self::Closed { at: at.clone() },
            _ => Self::Store(why),
        }
    }

    /// The same, for a backend whose failures are its own rather than the
    /// store's - a client talking to one over a wire. `doing` is the operation
    /// the wire was carrying, and what the transport said is the frame below.
    pub fn from_backend<E>(at: &StorePath, doing: StorageError, why: Report<E>) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::from_store(at, why.change_context(doing))
    }

    /// The refusal an interceptor gave.
    pub fn intercepted(at: &StorePath, said: impl AsRef<str>) -> Self {
        Self::Intercepted {
            at: at.clone(),
            said: Arc::from(said.as_ref()),
        }
    }
}

impl fmt::Display for WriteValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Intercepted { at, said } => {
                write!(f, "an interceptor turned down the write to {at}: {said}")
            }
            Self::Absent { at } => write!(f, "nothing is stored at {at}"),
            Self::NotAPath(why) => write!(f, "the write was given no path to land at: {why}"),
            Self::TooDeep { at, .. } => write!(f, "{at} is deeper than this store reads back"),
            Self::WillNotEncode { at, why } => {
                write!(
                    f,
                    "what was written to {at} will not encode: {}",
                    one_line(why)
                )
            }
            Self::Closed { at } => {
                write!(f, "the store was closed, so nothing was written to {at}")
            }
            Self::SourceGone => f.write_str(
                "the value this cell views is gone: the field or map it came from was dropped",
            ),
            Self::Store(why) => write!(f, "{}", why.current_context()),
        }
    }
}

impl std::error::Error for WriteValue {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Store(why) | Self::TooDeep { why, .. } | Self::WillNotEncode { why, .. } => {
                Some(why.current_context())
            }
            Self::Intercepted { .. }
            | Self::Absent { .. }
            | Self::Closed { .. }
            | Self::SourceGone => None,
        }
    }
}

impl From<StorePathError> for WriteValue {
    fn from(why: StorePathError) -> Self {
        Self::NotAPath(why)
    }
}

impl From<Report<StorageError>> for WriteValue {
    fn from(why: Report<StorageError>) -> Self {
        Self::Store(why)
    }
}

/// Back into a report, for the plumbing under the boundary.
///
/// The sets are what a caller outside the library meets; inside it, everything
/// still travels as a report, and this is what lets a `?` cross from one to
/// the other without a `map_err` at every line.
impl From<WriteValue> for Report<StorageError> {
    fn from(why: WriteValue) -> Self {
        match why {
            WriteValue::Store(report)
            | WriteValue::TooDeep { why: report, .. }
            | WriteValue::WillNotEncode { why: report, .. } => report,
            WriteValue::NotAPath(why) => Report::new(why).change_context(StorageError::Path),
            WriteValue::Closed { at } => {
                Report::new(StorageError::Closed).attach(crate::facts::Key(at))
            }
            WriteValue::Absent { at } => Report::new(StorageError::Read)
                .attach(crate::facts::Key(at))
                .attach("nothing is stored there"),
            WriteValue::Intercepted { at, said } => Report::new(StorageError::Write)
                .attach(crate::facts::Key(at))
                .attach(format!("an interceptor turned it down: {said}")),
            WriteValue::SourceGone => Report::new(StorageError::Write)
                .attach("the field or map this cell viewed was dropped"),
        }
    }
}

pub type WriteResult<T> = Result<T, WriteValue>;

pub type FieldError = WriteValue;
pub type ReactiveMapError = WriteValue;

pub type ReactiveFieldResult<T> = WriteResult<T>;
pub type ReactiveMapResult<T> = WriteResult<T>;
