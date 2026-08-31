use std::fmt;

/// Anything that can go wrong writing through a reactive primitive.
///
/// Fields and maps fail in exactly the same ways, so they share one error. The
/// per-primitive names below are aliases kept for readability at call sites.
///
/// This is a context, not a chain: what the engine underneath said is the frame
/// below this one in the report, and the particulars - which path, which file -
/// are attachments put there by whoever knew them. A variant carries data only
/// where that data *is* the failure rather than the circumstances of it.
#[derive(Debug, PartialEq, Eq)]
pub enum WriteError {
    /// The store refused the write or could not carry it out.
    Storage,

    /// An interceptor rejected the change.
    Intercepted,

    /// The addressed value is absent: a map key for maps, a store path for
    /// fields.
    KeyNotFound(String),

    /// A name that cannot be a level, so nothing can be stored under it.
    Path,

    /// The primitive a cell views has been dropped.
    SourceGone,

    /// A `Kv` write aimed at a path a declared struct owns.
    ///
    /// `declared` is the path the schema declared, which is either `path`
    /// itself or one it lies inside.
    SchemaOwned { path: String, declared: String },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Storage => f.write_str("the store could not carry out the write"),
            WriteError::Intercepted => f.write_str("an interceptor rejected the change"),
            WriteError::KeyNotFound(key) => write!(f, "nothing is stored at `{key}`"),
            WriteError::Path => f.write_str("a name that cannot be a level"),
            WriteError::SourceGone => f.write_str(
                "the value this cell views is gone: the field or map it came from was dropped (`into_cell` gives a cell that keeps it alive)",
            ),
            WriteError::SchemaOwned { path, declared } if path == declared => {
                write!(f, "`{path}` is declared by a schema")
            }
            WriteError::SchemaOwned { path, declared } if declared.len() > path.len() => {
                write!(f, "`{path}` holds `{declared}`, which a schema declares")
            }
            WriteError::SchemaOwned { path, declared } => {
                write!(f, "`{path}` is inside `{declared}`, which a schema declares")
            }
        }
    }
}

impl std::error::Error for WriteError {}

impl WriteError {
    /// The refusal an interceptor gave, kept as the frame below
    /// [`WriteError::Intercepted`].
    ///
    /// A filter turning a value down and interceptors recursing past the depth
    /// guard both end here, and only that sentence tells them apart - the
    /// second is a bug in the code that installed them.
    pub fn intercepted(why: String) -> error_stack::Report<Self> {
        error_stack::Report::new(WriteError::Intercepted).attach(why)
    }
}

pub type WriteResult<T> = Result<T, error_stack::Report<WriteError>>;

pub type FieldError = WriteError;
pub type ReactiveMapError = WriteError;

pub type ReactiveFieldResult<T> = WriteResult<T>;
pub type ReactiveMapResult<T> = WriteResult<T>;
