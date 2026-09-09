use crate::failure::{Because, StorageError};
use crate::path::{SmolStr, StorePath, StorePathError};
use error_stack::Report;
use std::fmt;

/// Everything a write through a reactive primitive can fail with, and nothing
/// else.
///
/// Fields, cells and maps fail in the same ways, so they share one set; the
/// per-primitive names below are aliases kept for readability at call sites.
///
/// Ordinary [`std::error::Error`], so a caller who does not want to look can
/// `?` it into `anyhow`, `eyre` or a `Box<dyn Error>` and be done - and get the
/// whole diagnosis out of it. [`Display`](fmt::Display) says what this write
/// was and where; [`source`](std::error::Error::source) descends through the
/// contexts under it, each carrying what was attached to it, so a
/// `{:?}` of an `anyhow::Error` names the operation, the key, the file and the
/// engine's own words without a `match` anywhere.
///
/// A caller who wants a fact as a type rather than as a sentence reads it back
/// with [`facts::all`](crate::facts::all) over the report the variant carries -
/// `facts::all::<StoreFile, _>(why)` for the file, `Key` for the key. That is
/// the typed way in; the chain is the readable one.
///
/// `{:?}` prints the same thing `anyhow` prints, because that is the shape a
/// reader has learnt: `fn main() -> anyhow::Result<()>` renders with `Debug`,
/// and so does every `tracing::error!("{err:?}")`. What the report holds beyond
/// the chain - the frame each context was raised at - is in
/// [`explain`](Self::explain), which is where a dump belongs.
pub enum WriteValue {
    /// An interceptor turned the change down, in its own words.
    Intercepted { at: StorePath, said: SmolStr },

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
    TooDeep { at: StorePath, why: Because },

    /// The value will not turn into what the store keeps, with what the codec
    /// said kept whole.
    WillNotEncode { at: StorePath, why: Because },

    /// The store has let go of its file, so nothing lands.
    ///
    /// No report, because there is never one to keep: every
    /// [`StorageError::Closed`] is minted fresh at the refusal - the store
    /// checks whether it is closed and answers - so nothing has failed
    /// underneath. What the report did carry is the store's own file, and that
    /// is dropped here rather than lost from somewhere deeper.
    Closed { at: StorePath },

    /// The field or map this cell views has been dropped. `into_cell` is the
    /// form that keeps it alive.
    SourceGone,

    /// The disk, in every sense: the file, the engine, the codec.
    ///
    /// Carries the report whole, so the facts attached along the way - the
    /// key, the table, how many bytes - are still there for whoever wants
    /// them.
    Store(Because),
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
                why: why.into(),
            },
            StorageError::Codec => Self::WillNotEncode {
                at: at.clone(),
                why: why.into(),
            },
            StorageError::Closed => Self::Closed { at: at.clone() },
            _ => Self::Store(why.into()),
        }
    }

    /// The whole report as a string, facts and all, for a caller handing this
    /// to `anyhow`: `.context(why.explain())`.
    ///
    /// The chain [`source`](std::error::Error::source) walks is the readable
    /// half and is usually enough. This is the rest of it - every frame, every
    /// attachment, laid out the way the report renders itself - for a log
    /// record that has to carry everything.
    pub fn explain(&self) -> String {
        match self {
            Self::Store(why) | Self::TooDeep { why, .. } | Self::WillNotEncode { why, .. } => {
                why.explain()
            }
            other => other.to_string(),
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
            said: SmolStr::new(said.as_ref()),
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
            Self::WillNotEncode { at, .. } => {
                write!(f, "what was written to {at} will not encode")
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

impl fmt::Debug for WriteValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        crate::failure::spelled(self, f)
    }
}

impl std::error::Error for WriteValue {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotAPath(why) => Some(why),
            Self::Store(why) | Self::TooDeep { why, .. } | Self::WillNotEncode { why, .. } => {
                why.caused().map(|under| under as &dyn std::error::Error)
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
        Self::Store(why.into())
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
            | WriteValue::WillNotEncode { why: report, .. } => report.into_report(),
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
