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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Claimed,

    /// The store was closed and has let go of its file.
    ///
    /// A close hands the file to whoever asked for it - another process, a
    /// backup, a rename - and it stays theirs, so every later read, write,
    /// scan and delete answers with this.
    Closed,
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
            StorageError::Claimed => "two schemas claim the same stored path",
            StorageError::Closed => "the store was closed and has let go of its file",
        })
    }
}

impl Error for StorageError {}

pub type StorageResult<T> = Result<T, Report<StorageError>>;

/// A write a document engine refuses because it cannot represent the result.
///
/// A tree holds a value at a node or values under it, never both, so the second
/// write is refused and the first survives. Only the document engines - json,
/// toml, ron - report this.
///
/// Sits below [`StorageError::Write`] so that a caller who has to tell this
/// apart - a seeding write, which nobody asked for, backs off where a real one
/// propagates - can do it without knowing which engine is underneath.
#[derive(Debug, PartialEq, Eq)]
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

/// What a migration step returns when it decides to fail.
///
/// A step is written by whoever uses the library, and the frames around it -
/// which prefix, which version, which store - are put there by the engine that
/// called it. So a step has nothing to add and only needs to say that this is
/// its refusal rather than the store's.
pub trait IntoStorageReport {
    fn into_report(self) -> Report<StorageError>;
}

impl IntoStorageReport for crate::MigrationError {
    fn into_report(self) -> Report<StorageError> {
        Report::new(self).change_context(StorageError::Migrate)
    }
}

impl IntoStorageReport for amethystate_core::path::StorePathError {
    fn into_report(self) -> Report<StorageError> {
        Report::new(self).change_context(StorageError::Path)
    }
}
