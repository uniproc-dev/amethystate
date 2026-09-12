use error_stack::Report;
use std::error::Error;
use std::fmt;

/// What a store operation failed at.
///
/// Each variant names the operation that failed. Which engine said what is the
/// frame below this one in the report, and the particulars - which path, which
/// file, which prefix - are attachments put there by whoever knew them. Two
/// engines failing to write are the same kind of failure, told apart by the
/// frames underneath.
///
/// `non_exhaustive` where the sets that carry it are not: this one is the
/// disk's own list and grows with the engines, and nobody is meant to match it
/// arm by arm. The sets are the opposite - a `match` over one is meant to be
/// complete, and gaining a way to fail is meant to break it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StorageError {
    /// Opening or creating the store.
    Open,

    /// Reading a value.
    Read,

    /// Writing a value.
    Write,

    /// Removing a key or a subtree.
    Delete,

    /// Listing what is under a prefix.
    Scan,

    /// Getting what is buffered onto disk.
    Flush,

    /// Turning a value into the store's format, or reading it back.
    Codec,

    /// Reading or writing the schema bookkeeping - versions, snapshots, the
    /// migration log, the initialization markers.
    Meta,

    /// Bringing stored data up to the schema the code declares.
    Migrate,

    /// A name that cannot be a level, so nothing can be stored under it.
    Path,

    /// A path or a value that nests deeper than this store reads back.
    ///
    /// Told apart from [`StorageError::Path`] because the names are all fine:
    /// what is wrong is how many of them there are, and the facts say which
    /// budget ran out and by how much.
    Depth,

    /// The flush this commit was waiting on did not complete.
    CommitFailed,

    /// Two owners want the same place, so one would write over the other.
    Taken,

    /// The store was closed and has let go of its file.
    ///
    /// A close hands the file to whoever asked for it - another process, a
    /// backup, a rename - and it stays theirs, so every later read, write,
    /// scan and delete answers with this.
    Closed,

    /// The change was stored, and somebody subscribed to it could not take it.
    ///
    /// The write landed: this is about what happened while telling people. A
    /// field that cannot read back what was just written to its path answers
    /// with this, which is how the writer finds out that what it stored is not
    /// what that field holds.
    Notify,

    /// Asked for from inside something the store is already doing, where doing
    /// it would mean waiting for the caller to finish.
    ///
    /// Closing from `on_persist_failure` is the one that reaches here: that
    /// callback runs on the thread a close has to wait for.
    Reentrant,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StorageError::Open => "the store could not be opened",
            StorageError::Read => "the store could not read",
            StorageError::Write => "the store could not write",
            StorageError::Delete => "the store could not delete",
            StorageError::Scan => "the store could not list a prefix",
            StorageError::Flush => "the store could not commit what it had buffered",
            StorageError::Codec => "the value could not be encoded or decoded",
            StorageError::Meta => "the schema bookkeeping could not be read or written",
            StorageError::Migrate => "the data could not be brought up to the declared schema",
            StorageError::Path => "a name that cannot be a level",
            StorageError::Depth => "deeper than this store reads back",
            StorageError::CommitFailed => "the flush this commit was waiting on did not complete",
            StorageError::Taken => "two schemas want the same place",
            StorageError::Closed => "the store was closed and has let go of its file",
            StorageError::Notify => "the change was stored, and a subscriber could not take it",
            StorageError::Reentrant => {
                "the store cannot do this from inside what it is already doing"
            }
        })
    }
}

impl Error for StorageError {}

pub type StorageResult<T> = Result<T, Report<StorageError>>;

/// A write a document engine refuses because its tree cannot represent the
/// result.
///
/// A tree holds a value at a node or values under it, never both, so the second
/// write is refused and the first survives. Only the document engines - json,
/// toml, ron - report this, and only inside the part of the file a schema
/// declares: what nothing declares is written whole, one key to a path, where
/// there is no level for a value to be at odds with.
///
/// Sits below [`StorageError::Write`] so that a caller who has to tell this
/// apart - a seeding write, which nobody asked for, backs off where a real one
/// propagates - can do it without knowing which engine is underneath.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Occupied {
    /// A value is stored at a level the write needs as a branch.
    Value { level: String },

    /// Values are stored under the level a plain value would replace.
    ///
    /// Only a value that is not itself a map is refused here. A serialized
    /// struct is a map with children, indistinguishable in a document from a
    /// level with values under it, so writing one over another is taken as the
    /// update it almost always is.
    Branch { level: String },
}

impl fmt::Display for Occupied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Occupied::Value { level } => {
                write!(
                    f,
                    "`{level}` holds a value, so nothing can be stored under it"
                )
            }
            Occupied::Branch { level } => write!(
                f,
                "`{level}` holds values under it, so a value cannot be stored at it"
            ),
        }
    }
}

impl Error for Occupied {}

/// A report's chain of contexts on one line, for a log record.
///
/// A report's `Display` shows only the outermost context, which is the one that
/// says least: "the store could not write" without what refused it. This keeps
/// the causes and drops the attachments, so the line stays greppable.
pub fn one_line<C: fmt::Display + Send + Sync + 'static>(report: &Report<C>) -> String {
    report
        .frames()
        .filter_map(|frame| match frame.kind() {
            error_stack::FrameKind::Context(context) => Some(context.to_string()),
            error_stack::FrameKind::Attachment(_) => None,
        })
        .collect::<Vec<_>>()
        .join(" <- ")
}

/// A report's frames as an ordinary error chain.
///
/// A [`Report`] keeps its frames itself, and [`Error::source`] has to hand back
/// something with a life of its own, so the chain is copied out where a report
/// crosses into a set. One link per context, carrying the attachments that were
/// put on it - the key, the file, the engine's own words - so a caller who
/// walks the chain and prints it gets what the report holds without knowing
/// there is a report.
///
/// The outermost context is left out: whoever carries this says it already, and
/// printing it again is the duplicate line this exists to remove. What was
/// attached to it is kept and moves down onto the first link that is a cause,
/// so no link is facts with nothing that failed above them - a reader walking
/// causes should meet causes. Only where the outermost is the sole context do
/// the facts stand on their own, because then there is nothing to move them to.
#[derive(Debug)]
pub struct Caused {
    said: String,
    under: Option<Box<Caused>>,
}

impl Caused {
    /// The chain under `report`'s outermost context, or `None` when the
    /// outermost is all there is and carries nothing.
    pub fn under<C>(report: &Report<C>) -> Option<Self> {
        let mut links: Vec<Link> = Vec::new();
        let mut waiting: Vec<String> = Vec::new();

        for frame in report.frames() {
            match frame.kind() {
                error_stack::FrameKind::Context(context) => links.push(Link {
                    said: Some(context.to_string()),
                    carried: std::mem::take(&mut waiting),
                }),
                error_stack::FrameKind::Attachment(error_stack::AttachmentKind::Printable(
                    shown,
                )) => waiting.push(shown.to_string()),
                _ => {}
            }
        }

        match links.len() {
            0 => {}
            1 => links[0].said = None,
            _ => {
                let outermost = links.remove(0);
                let mut carried = outermost.carried;
                carried.append(&mut links[0].carried);
                links[0].carried = carried;
            }
        }

        links
            .into_iter()
            .rev()
            .filter(|link| !link.is_empty())
            .fold(None, |under, link| {
                Some(Caused {
                    said: link.spelled(),
                    under: under.map(Box::new),
                })
            })
    }
}

/// One context and what was attached to it, before it is spelled out.
struct Link {
    said: Option<String>,
    carried: Vec<String>,
}

impl Link {
    fn is_empty(&self) -> bool {
        self.said.is_none() && self.carried.is_empty()
    }

    fn spelled(self) -> String {
        match (self.said, self.carried.is_empty()) {
            (Some(said), true) => said,
            (Some(said), false) => format!("{said} ({})", self.carried.join("; ")),
            (None, _) => self.carried.join("; "),
        }
    }
}

impl fmt::Display for Caused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.said)
    }
}

impl Error for Caused {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.under.as_deref().map(|under| under as &dyn Error)
    }
}

/// A set's own sentence and the chain under it, laid out the way `anyhow` lays
/// one out.
///
/// What every set's [`fmt::Debug`] is. A library error and the `anyhow::Error`
/// it becomes should not print two different ways: whoever wrote `{err:?}`
/// learnt the shape from one of them, and `fn main() -> anyhow::Result<()>`
/// renders with `Debug`.
pub fn spelled<E>(why: &E, f: &mut fmt::Formatter<'_>) -> fmt::Result
where
    E: Error + ?Sized,
{
    write!(f, "{why}")?;

    let mut under = why.source();

    if under.is_some() {
        f.write_str("\n\nCaused by:")?;
    }

    while let Some(step) = under {
        write!(f, "\n    {step}")?;
        under = step.source();
    }

    Ok(())
}

/// A report on its way out of the library, with the chain a caller can walk.
///
/// The report is still there whole - [`Deref`](std::ops::Deref) reaches it, and
/// so do [`facts::all`](crate::facts::all) and a `{:?}` of it - and beside it
/// sits the same thing as an ordinary error chain, built the first time
/// somebody asks and not before. A failure that nobody looks into costs the
/// report it was already carrying and nothing more.
pub struct Because {
    why: Report<StorageError>,
    chain: std::sync::OnceLock<Option<Caused>>,
}

impl Because {
    pub fn new(why: Report<StorageError>) -> Self {
        Self {
            why,
            chain: std::sync::OnceLock::new(),
        }
    }

    /// The report, for the plumbing under the boundary.
    pub fn into_report(self) -> Report<StorageError> {
        self.why
    }

    /// What is under the outermost context, as an error chain.
    pub fn caused(&self) -> Option<&Caused> {
        self.chain.get_or_init(|| Caused::under(&self.why)).as_ref()
    }

    /// The whole report as a string, facts and all.
    ///
    /// For a caller who is handing this to `anyhow` and wants everything in one
    /// place: `.context(why.explain())`.
    pub fn explain(&self) -> String {
        format!("{:?}", self.why)
    }
}

impl std::ops::Deref for Because {
    type Target = Report<StorageError>;

    fn deref(&self) -> &Self::Target {
        &self.why
    }
}

impl fmt::Debug for Because {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.why, f)
    }
}

/// What the report says, which is its outermost context. The chain under it is
/// [`caused`](Self::caused) and the whole of it is [`explain`](Self::explain).
impl fmt::Display for Because {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.why, f)
    }
}

impl From<Report<StorageError>> for Because {
    fn from(why: Report<StorageError>) -> Self {
        Self::new(why)
    }
}

/// What a migration step returns when it decides to fail.
///
/// A step is written by whoever uses the library, and the frames around it -
/// which prefix, which version, which store - are put there by the engine that
/// called it. So a step has nothing to add and only needs to say that this is
/// its refusal rather than the store's.
pub trait IntoStorageReport {
    fn into_report(self) -> Report<StorageError>;
}

impl IntoStorageReport for crate::path::StorePathError {
    fn into_report(self) -> Report<StorageError> {
        Report::new(self).change_context(StorageError::Path)
    }
}
