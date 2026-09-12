use crate::codec::CodecError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MigrationError {
    #[error(transparent)]
    Codec(#[from] CodecError),

    #[error(
        "Migration chain gap for [{prefix}]: reached v{reached_version}, expected v{expected_version}"
    )]
    Gap {
        prefix: String,
        reached_version: u32,
        expected_version: u32,
    },

    /// A step reached into a prefix whose own migration is already running, so
    /// neither can go first. The whole chain is named, outermost first, ending
    /// on the prefix that closed it.
    #[error("a migration reached round to where it started: {}", .0.join(" -> "))]
    Cycle(Vec<String>),

    #[error("Migration error: {0}")]
    Custom(String),

    /// A step reached into a prefix that holds keys nothing records a version
    /// for. There is no telling which steps have already run over them, and
    /// running them again is worse than not running them at all.
    #[error(
        "a migration reached into [{prefix}], which holds keys nothing records a version for: \
         the bookkeeping that would say which steps have run is gone"
    )]
    VersionUnknown { prefix: String },

    /// What this step would record at the prefix says two things at once: two
    /// declarations at one version, both owning the same place. Which of them
    /// owns it is then whichever is looked at first, so the step is rolled back
    /// rather than written.
    #[error("[{prefix}] would be recorded saying two things at once: {said}")]
    Contradiction { prefix: String, said: String },

    #[error("Downgrade detected for [{prefix}]: DB v{db_version}, Code v{code_version}")]
    Downgrade {
        prefix: String,
        db_version: u32,
        code_version: u32,
    },
}
