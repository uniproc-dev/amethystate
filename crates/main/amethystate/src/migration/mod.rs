use tracing::{info, warn};

pub mod builder;
pub mod context;
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
pub struct SchemaDiff {
    pub added: Vec<meta::StoredFieldEntry>,
    pub removed: Vec<meta::StoredFieldEntry>,
}

#[derive(Debug, Clone)]
pub struct NaggingRecord {
    pub prefix: String,
    pub diff: Option<SchemaDiff>,

    /// Every difference between the places declared last time and the places
    /// declared now, with what each one amounts to.
    ///
    /// What raised the complaint is in here: the record exists because one of
    /// these breaks, and the rest are carried so a person can see the change
    /// whole rather than the one part of it that failed.
    pub moved: Vec<Moved>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppliedStep {
    pub prefix: String,
    pub target_version: u32,
    pub description: Option<String>,
    pub applied_at: u64,
}

#[derive(Debug, Default)]
pub struct MigrationReport {
    pub components: Vec<ComponentResult>,
}

#[derive(Debug)]
pub struct ComponentResult {
    /// Everything this pass held: the prefix it started at and every one a
    /// step reached from there.
    pub prefixes: Vec<StorePath>,
    pub outcome: ComponentOutcome,
    pub nagging: Vec<NaggingRecord>,
}

#[derive(Debug)]
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
pub enum NotMigrated {
    /// The store already holds what the code declares.
    UpToDate,

    /// The prefix holds keys and nothing records what version they are at.
    ///
    /// The version rides with the declaration the store wrote down, and both
    /// live in the bookkeeping; a prefix that has keys and neither is one
    /// whose bookkeeping was lost. Which steps have already run over those
    /// keys is then unknowable, and running them again is the worse of the two
    /// answers - so they are left where they are and this says so.
    VersionUnknown,
}

impl MigrationReport {
    /// Whether any step failed. A failure leaves that prefix at its old
    /// version, with a snapshot kept for the next run.
    pub fn has_failures(&self) -> bool {
        self.components
            .iter()
            .any(|c| matches!(c.outcome, ComponentOutcome::Failed { .. }))
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
    /// [`StoreBuilder::build_with_migration`](crate::StoreBuilder::build_with_migration)
    /// already does this, so calling it again duplicates the lines.
    pub fn log_to_tracing(&self) {
        for comp in &self.components {
            for nag in &comp.nagging {
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
                    tracing::debug!("⏩ Component [{}] is up to date", Self::named(&comp.prefixes));
                }
                ComponentOutcome::Skipped(NotMigrated::VersionUnknown) => {
                    warn!(
                        "⚠️  Component [{}] holds keys and nothing records what version they are, \
                         so it was left as it is: the bookkeeping that would say which steps have \
                         run is gone, and running them again could apply a step twice",
                        Self::named(&comp.prefixes)
                    );
                }
            }
        }
    }
}

pub trait Migration: Send + Sync {
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
