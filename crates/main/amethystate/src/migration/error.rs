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

    /// Two declarations of one line at one version. Versions are told apart
    /// by their number and nothing else, so this is one version said twice -
    /// most often two structs sharing a prefix where each needed an `id`.
    #[error(
        "[{lineage}] is declared twice at version {version}, by `{}` and by `{}`: two structs \
         sharing a prefix are two lines of declarations, and each needs an `id` of its own",
        .between.0,
        .between.1
    )]
    DeclaredTwice {
        lineage: String,
        version: u32,
        between: (&'static str, &'static str),
    },

    /// Two lines at one prefix own one place, so what is stored there would be
    /// read in whichever shape the reader met first.
    #[error(
        "`{at}` is owned by two lines of declarations, `{}` and `{}`: a place holds one thing, so \
         one of them has to give it up",
        .between.0,
        .between.1
    )]
    ClaimedTwice {
        at: String,
        between: (&'static str, &'static str),
    },

    /// Two steps of one line take it to the same version, so running one and
    /// passing over the other would decide between them by the order they were
    /// handed over.
    #[error(
        "[{lineage}] has two steps to v{version}, `{}` and `{}`: one version is reached by one \
         step",
        .between.0,
        .between.1
    )]
    StepTwice {
        lineage: String,
        version: u32,
        between: (String, String),
    },

    /// A merge found one of its two sources and not the other, so it could
    /// neither combine them nor leave the store as a merge of nothing leaves it.
    #[error("[{prefix}] merges `{first}` and `{second}`, and only `{present}` is there")]
    HalfAMerge {
        prefix: String,
        first: String,
        second: String,
        present: String,
    },

    /// Steps take a line past the version its struct declares, so the shape
    /// they leave is one nothing declares, and a step written later for that
    /// version would be passed over as already run.
    #[error(
        "[{lineage}] has a step to v{planned} and its struct declares v{declared}: nothing declares \
         the shape that step would leave"
    )]
    PlanPastDeclared {
        lineage: String,
        planned: u32,
        declared: u32,
    },

    /// The bookkeeping holds two records for one line, so which of them says
    /// what is on disk cannot be told.
    #[error("[{lineage}] is recorded twice, and which record is the shape on disk cannot be told")]
    RecordedTwice { lineage: String },

    #[error("Downgrade detected for [{prefix}]: DB v{db_version}, Code v{code_version}")]
    Downgrade {
        prefix: String,
        db_version: u32,
        code_version: u32,
    },
}
