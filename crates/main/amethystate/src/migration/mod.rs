use tracing::{info, warn};

pub mod builder;
pub mod context;
#[cfg(feature = "diagnostics")]
pub mod diagnostic;
pub mod engine;
pub mod error;
pub mod fields;
pub mod migrate_from;
pub mod node;
pub mod provided;
pub mod registry;
pub mod set;
pub mod step;

use crate::store::moved::Moved;
use crate::store::{StorageError, meta, one_line};
use amethystate_core::path::StorePath;
pub use context::MigrationContext;
pub use error::MigrationError;
pub use step::{RunStep, StepResult};

/// Which declared paths a store holds that the code does not, and the other way
/// round.
///
/// By name only. What a path *is* - its role, whether it may hold nothing, what
/// lives under it - is recorded per field in the snapshot and is not compared
/// here; that comparison is a diff of two schema documents, which this is not.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct SchemaDiff {
    pub added: Vec<meta::StoredFieldEntry>,
    pub removed: Vec<meta::StoredFieldEntry>,
}

/// One prefix holding a shape the code does not declare, and everything there
/// is to say about it.
///
/// `non_exhaustive`, like the rest of the report: what is worth telling somebody
/// about drift is the part of this library most likely to gain a fact, and a
/// reader takes the fields it wants rather than destructuring the lot.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NaggingRecord {
    pub prefix: StorePath,

    /// The line at the prefix that drifted, `None` for the unnamed one.
    pub id: Option<String>,

    pub diff: Option<SchemaDiff>,

    /// Every difference between the places declared last time and the places
    /// declared now, with what each one amounts to.
    ///
    /// What raised the complaint is in here: the record exists because one of
    /// these breaks, and the rest are carried so a person can see the change
    /// whole rather than the one part of it that failed.
    pub moved: Vec<Moved>,
}

/// One step that ran, as the migration log records it.
///
/// `non_exhaustive`: this is written to disk and read back, so a build that
/// gains a field here still reads what an older one wrote.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct AppliedStep {
    pub prefix: String,

    /// The line of declarations at the prefix the step moved, `None` for the
    /// unnamed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    pub target_version: u32,
    pub description: Option<String>,
    pub applied_at: u64,
}

/// What a migration pass did, prefix by prefix.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct MigrationReport {
    pub components: Vec<ComponentResult>,
}

/// One transaction's worth of it.
#[derive(Debug)]
#[non_exhaustive]
pub struct ComponentResult {
    /// Everything this pass held: the prefix it started at and every one a
    /// step reached from there.
    pub prefixes: Vec<StorePath>,
    pub outcome: ComponentOutcome,
    pub nagging: Vec<NaggingRecord>,
}

/// How it ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum ComponentOutcome {
    Committed {
        steps: Vec<AppliedStep>,
    },
    Skipped(NotMigrated),
    Failed {
        error: error_stack::Report<StorageError>,
    },
}

/// Why a prefix was left as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotMigrated {
    /// The store already holds what the code declares.
    UpToDate,

    /// The prefix held keys and nothing recorded what version they were at, so
    /// they were taken to stand at the version the code declares - `taken_as` -
    /// and no step ran over them.
    ///
    /// The version rides with the declaration the store wrote down, and both
    /// live in the bookkeeping; a prefix that has keys and neither is one whose
    /// bookkeeping was lost. Which steps have already run over those keys is
    /// then unknowable, and running them again is the worse of the two answers:
    /// a step is written to be run once, and a second pass over data it already
    /// moved is how a rename becomes two renames.
    ///
    /// So the code's own version is what dates them. A binary reads the store it
    /// wrote, and the shape it declares is the shape that wrote those keys -
    /// which is an assumption rather than a fact, and the reason this is said
    /// out loud at `warn` rather than passed over.
    ///
    /// It is decided once, on the open that meets the loss. The version is
    /// written down there and then, so every open after it is ordinary.
    BookkeepingLost { taken_as: u32 },
}

impl NaggingRecord {
    /// The line that drifted.
    pub fn lineage(&self) -> crate::schema::Lineage {
        crate::schema::Lineage {
            prefix: self.prefix.clone(),
            id: self.id.clone(),
        }
    }
}

impl MigrationReport {
    /// Whether any step failed. A failure leaves that prefix at its old
    /// version, with a snapshot kept for the next run.
    pub fn has_failures(&self) -> bool {
        self.components
            .iter()
            .any(|c| matches!(c.outcome, ComponentOutcome::Failed { .. }))
    }

    /// Every failure the pass ended with, in the order the prefixes ran.
    pub fn failures(&self) -> impl Iterator<Item = &error_stack::Report<StorageError>> {
        self.components.iter().filter_map(|c| match &c.outcome {
            ComponentOutcome::Failed { error } => Some(error),
            _ => None,
        })
    }
    /// Whether stored data differs in shape from what the structs now
    /// declare, without a step to account for it - the sign of a schema
    /// change someone forgot to write a migration for.
    pub fn has_drift(&self) -> bool {
        self.components.iter().any(|c| !c.nagging.is_empty())
    }

    /// The prefixes a pass held, spelled the way a reader would name them.
    fn named(prefixes: &[StorePath]) -> String {
        prefixes
            .iter()
            .map(StorePath::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Writes the report through `tracing`, at a level per outcome.
    ///
    /// [`StoreBuilder::build`](crate::StoreBuilder::build) and
    /// [`StoreBuilder::migrate`](crate::StoreBuilder::migrate) already do this,
    /// so calling it again duplicates the lines.
    ///
    /// With the `diagnostics` feature on, drift is written as
    /// [`drift`](MigrationReport::drift) renders it - one laid-out warning per
    /// prefix - instead of a line per changed field. Everything else reads the
    /// same either way.
    pub fn log_to_tracing(&self) {
        self.nagging_to_tracing();

        for comp in &self.components {
            match &comp.outcome {
                ComponentOutcome::Committed { steps } => {
                    for step in steps {
                        info!(
                            "✅ Applied: {} v{} ({})",
                            step.prefix,
                            step.target_version,
                            step.description.as_deref().unwrap_or("no description")
                        );
                    }
                }
                ComponentOutcome::Failed { error } => {
                    tracing::error!(
                        "❌ Component [{}] failed: {}",
                        Self::named(&comp.prefixes),
                        one_line(error)
                    );
                    tracing::error!(
                        "   Transaction rolled back. Data for these prefixes remains unchanged."
                    );
                }
                ComponentOutcome::Skipped(NotMigrated::UpToDate) => {
                    tracing::debug!(
                        "⏩ Component [{}] is up to date",
                        Self::named(&comp.prefixes)
                    );
                }
                ComponentOutcome::Skipped(NotMigrated::BookkeepingLost { taken_as }) => {
                    warn!(
                        "⚠️  Component [{}] held keys and nothing recorded what version they were \
                         at, so they are taken to stand at v{taken_as} - what this build declares \
                         - and no step ran over them. The bookkeeping that would have said which \
                         steps had already run is gone, and running them again is how one rename \
                         becomes two. Check the data if this store was written by an older build",
                        Self::named(&comp.prefixes)
                    );
                }
            }
        }
    }

    #[cfg(not(feature = "diagnostics"))]
    fn nagging_to_tracing(&self) {
        for nag in self.components.iter().flat_map(|comp| comp.nagging.iter()) {
            warn!("⚠️  Schema drift detected in prefix '{}'", nag.prefix);

            if let Some(diff) = &nag.diff {
                for f in &diff.added {
                    warn!("  + field '{}': {}", f.name, f.type_name);
                }
                for f in &diff.removed {
                    warn!("  - field '{}' (exists in DB, missing in code)", f.name);
                }
            }

            warn!(
                "  Suggestion: increment version and write a migration if these changes are intentional."
            );
        }
    }

    #[cfg(feature = "diagnostics")]
    fn nagging_to_tracing(&self) {
        for one in self.drift() {
            warn!("{}", diagnostic::rendered(&one));
        }
    }
}

/// One step, as a [`MigrationPlan`] holds it.
///
/// Nothing outside this crate implements it, and nothing outside could use an
/// implementation: a plan's steps are its own, and the way into one is
/// [`MigrationPlan::step`], which takes the closure and wraps it. The trait is
/// how a plan holds steps of different shapes side by side, not an extension
/// point.
pub(crate) trait Migration: Send + Sync {
    fn target_version(&self) -> u32;
    fn description(&self) -> Option<&str> {
        None
    }
    fn run(&self, ctx: &mut MigrationContext) -> StepResult<()>;
}

pub struct MigrationPlan {
    pub(crate) steps: Vec<Box<dyn Migration>>,
}

impl MigrationPlan {
    /// An empty plan, to be filled with [`MigrationPlan::step`].
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Adds a step taking the data to `version`, and yields the plan back for
    /// chaining.
    ///
    /// Steps run in ascending version order, and only those above the version
    /// the prefix currently records.
    pub fn step<F>(mut self, version: u32, description: &str, f: F) -> Self
    where
        F: Fn(&mut MigrationContext) -> StepResult<()> + Send + Sync + 'static,
    {
        struct ClosureMigration<F> {
            v: u32,
            d: String,
            f: F,
        }
        impl<F> Migration for ClosureMigration<F>
        where
            F: Fn(&mut MigrationContext) -> StepResult<()> + Send + Sync + 'static,
        {
            fn target_version(&self) -> u32 {
                self.v
            }
            fn description(&self) -> Option<&str> {
                Some(&self.d)
            }
            fn run(&self, ctx: &mut MigrationContext) -> StepResult<()> {
                (self.f)(ctx)
            }
        }

        self.steps.push(Box::new(ClosureMigration {
            v: version,
            d: description.to_string(),
            f,
        }));
        self.steps.sort_by_key(|s| s.target_version());
        self
    }
}

impl Default for MigrationPlan {
    fn default() -> Self {
        Self::new()
    }
}
