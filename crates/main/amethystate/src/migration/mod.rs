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
pub mod types;

use crate::store::{StorageError, StorageResult, meta, one_line};
pub use context::MigrationContext;
pub use error::MigrationError;

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
    pub old_hash: u32,
    pub new_hash: u32,
    pub diff: Option<SchemaDiff>,
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
    pub prefixes: Vec<String>,
    pub outcome: ComponentOutcome,
    pub nagging: Vec<NaggingRecord>,
}

#[derive(Debug)]
pub enum ComponentOutcome {
    Committed {
        steps: Vec<AppliedStep>,
    },
    Skipped,
    Failed {
        error: error_stack::Report<StorageError>,
    },
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
                        "❌ Component {:?} failed: {}",
                        comp.prefixes,
                        one_line(error)
                    );
                    tracing::error!(
                        "   Transaction rolled back. Data for these prefixes remains unchanged."
                    );
                }
                ComponentOutcome::Skipped => {
                    tracing::debug!("⏩ Component {:?} is up to date", comp.prefixes);
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
    fn run(&self, ctx: &mut MigrationContext) -> StorageResult<()>;
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
        F: Fn(&mut MigrationContext) -> StorageResult<()> + Send + Sync + 'static,
    {
        struct ClosureMigration<F> {
            v: u32,
            d: String,
            f: F,
        }
        impl<F> Migration for ClosureMigration<F>
        where
            F: Fn(&mut MigrationContext) -> StorageResult<()> + Send + Sync + 'static,
        {
            fn target_version(&self) -> u32 {
                self.v
            }
            fn description(&self) -> Option<&str> {
                Some(&self.d)
            }
            fn run(&self, ctx: &mut MigrationContext) -> StorageResult<()> {
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
