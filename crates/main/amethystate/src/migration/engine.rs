use crate::migration::context::Reaching;
use crate::migration::fields::FieldDescriptor;
use crate::migration::meta::{SchemaSnapshot, StoredFieldEntry};
use crate::migration::set::MigrationSet;
use crate::migration::{
    AppliedStep, ComponentOutcome, ComponentResult, NaggingRecord, NotMigrated, SchemaDiff,
};
use crate::schema::Lineage;
use crate::store::MigrationBackendAdapter;
use crate::store::moved::{self, Moved, Verdict};
use crate::store::{StorageError, StorageResult};
use crate::{MigrationContext, MigrationError, MigrationPlan, MigrationReport};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

pub trait StorageProvider {
    fn atomic<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&mut dyn MigrationBackendAdapter) -> StorageResult<T>;
}

pub struct MigrationEngine<'a, P: StorageProvider> {
    provider: &'a P,
}

/// Drops the records a run of steps has just answered for.
///
/// A record nothing declares any more is what raises drift: the places it held
/// are under no declaration, and somebody has to say what happens to them.
/// Taking the prefix past the version that record stands at *is* somebody saying
/// so, in the words the drift report asks for it. Nothing retired the record
/// afterwards, so the same complaint came back on every open for ever.
///
/// Only below the new version, and only the line's own records and the ones no
/// line standing at the prefix answers for. A record at the version the line has
/// just arrived at is the shape it arrived *as*, and a record of another line
/// some struct still declares is that struct's.
fn retire_what_the_steps_answered_for(
    recorded: &mut Vec<SchemaSnapshot>,
    lineage: &Lineage,
    arrived_at: u32,
) {
    let standing = crate::schema::current_at(&lineage.prefix);

    recorded.retain(|was| {
        let own = was.id.as_deref() == lineage.id();
        let someone_elses = !own && standing.iter().any(|entry| entry.id == was.id.as_deref());

        was.version >= arrived_at || someone_elses
    });
}

/// What a pass will not write a schema snapshot for.
///
/// Two lists because the reasons are two sizes. A migration that failed and a
/// prefix nothing could date are facts about the whole prefix: whatever is
/// declared there, the store's shape is not settled. Drift is a fact about one
/// declaration, and a second declaration at the same prefix has done nothing
/// wrong - holding it back too would leave it unrecorded for ever, with no way
/// of its own to make the neighbour's drift stop.
#[derive(Default)]
pub struct HeldBack {
    prefixes: Vec<StorePath>,
    declarations: Vec<&'static [FieldDescriptor]>,
}

impl HeldBack {
    fn covers(&self, entry: &crate::schema::SchemaEntry) -> bool {
        self.prefixes.contains(&entry.prefix)
            || self
                .declarations
                .iter()
                .any(|held| std::ptr::eq(*held, entry.fields))
    }
}

/// One transaction's worth of migrating: the prefix it started at, and every
/// prefix a step reached into from there.
///
/// The stack is what a cycle runs into. A prefix on it is one whose steps are
/// part-way through, so a reach back into it cannot be answered - neither can
/// go first - and the chain is named end to end rather than at the one link
/// that closed it.
struct Pass<'a, P: StorageProvider> {
    engine: &'a MigrationEngine<'a, P>,
    mset: &'a MigrationSet,

    /// Lines an earlier pass already committed.
    settled: &'a HashSet<Lineage>,

    covered: RefCell<Vec<Lineage>>,
    running: RefCell<Vec<Lineage>>,
    steps: RefCell<Vec<AppliedStep>>,
    nagging: RefCell<Vec<NaggingRecord>>,
}

impl<'a, P: StorageProvider> Pass<'a, P> {
    fn new(
        engine: &'a MigrationEngine<'a, P>,
        mset: &'a MigrationSet,
        settled: &'a HashSet<Lineage>,
    ) -> Self {
        Self {
            engine,
            mset,
            settled,
            covered: RefCell::new(Vec::new()),
            running: RefCell::new(Vec::new()),
            steps: RefCell::new(Vec::new()),
            nagging: RefCell::new(Vec::new()),
        }
    }

    /// Whether `lineage` is at a version or a shape the code no longer agrees
    /// with.
    fn needs_work(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        lineage: &Lineage,
    ) -> StorageResult<bool> {
        let current_v = storage
            .get_meta(&lineage.prefix)?
            .and_then(|meta| meta.version_of(lineage.id()))
            .unwrap_or(0);
        let (target_v, target_fields) = self.mset.get_target(lineage);

        Ok(target_v != current_v
            || !self
                .engine
                .places_that_moved(storage, lineage, target_fields)?
                .is_empty())
    }

    /// Whether `prefix` holds keys that nothing can date.
    ///
    /// The version a prefix stands at is in [`PrefixMeta`], which lives in the
    /// store's bookkeeping; where that is gone and the keys are not, there is
    /// no telling which steps have already run over them, and running them
    /// again is the worse of the two answers. A prefix with no keys has
    /// nothing to run them over and is the ordinary first open.
    ///
    /// [`PrefixMeta`]: crate::store::meta::PrefixMeta
    fn version_is_lost(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        prefix: &StorePath,
    ) -> StorageResult<bool> {
        if !storage.bookkeeping_is_lost() {
            return Ok(false);
        }

        Ok(!storage.scan_prefix(prefix)?.is_empty())
    }

    fn bring_up_to_date(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        lineage: &Lineage,
    ) -> StorageResult<()> {
        if self.running.borrow().iter().any(|at| at == lineage) {
            let mut chain: Vec<String> = self
                .running
                .borrow()
                .iter()
                .map(Lineage::to_string)
                .collect();
            chain.push(lineage.to_string());

            return Err(
                Report::new(MigrationError::Cycle(chain)).change_context(StorageError::Migrate)
            );
        }

        if self.settled.contains(lineage) || self.covered.borrow().iter().any(|at| at == lineage) {
            return Ok(());
        }

        if self.version_is_lost(storage, &lineage.prefix)? {
            return Err(Report::new(MigrationError::VersionUnknown {
                prefix: lineage.to_string(),
            })
            .change_context(StorageError::Migrate));
        }

        self.covered.borrow_mut().push(lineage.clone());
        self.running.borrow_mut().push(lineage.clone());

        let ran = self
            .engine
            .migrate_prefix(storage, lineage, self.mset, self)
            .map_err(|why| {
                if why.contains::<crate::store::facts::Migrating>() {
                    why
                } else {
                    why.attach(crate::store::facts::Migrating(lineage.to_string()))
                }
            });
        self.running.borrow_mut().pop();

        let (steps, nagging) = ran?;
        self.steps.borrow_mut().extend(steps);
        self.nagging.borrow_mut().extend(nagging);

        Ok(())
    }

    fn covered(&self) -> Vec<Lineage> {
        self.covered.borrow().clone()
    }

    fn steps(&self) -> Vec<AppliedStep> {
        self.steps.borrow().clone()
    }

    fn nagging(&self) -> Vec<NaggingRecord> {
        self.nagging.borrow().clone()
    }
}

impl<P: StorageProvider> Reaching for Pass<'_, P> {
    fn reach(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        from: &StorePath,
        key: &StorePath,
    ) -> StorageResult<()> {
        let Some(owner) = self.mset.owner_of(key) else {
            return Ok(());
        };

        let reaching = self.running.borrow().last().cloned();

        for lineage in self.mset.lineages_at(&owner) {
            if reaching.as_ref() == Some(&lineage) {
                continue;
            }

            self.bring_up_to_date(storage, &lineage)
                .attach_with(|| format!("reached from {from} into {key}"))?;
        }

        Ok(())
    }
}

impl<'a, P: StorageProvider> MigrationEngine<'a, P> {
    pub fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    /// Records the schema the code declares, so a later run can tell what
    /// changed under it.
    ///
    /// What [`HeldBack`] covers keeps the snapshot it has, for one reason in
    /// three shapes: recording the current schema would say something about the
    /// store that nothing established.
    ///
    /// A migration that failed is one - overwriting its snapshot leaves the next
    /// run nothing to compare against, and the diagnostic for the one prefix
    /// that needs it is gone for good. A prefix whose version is unknown is the
    /// plainest: a shape may not be written down for keys nothing could date.
    /// Drift nobody answered is the third - it is raised because the store holds
    /// a shape the code does not declare, and recording the declaration makes
    /// that stop being true without anything having been done about it.
    ///
    /// A declaration at a version below what is already recorded is skipped
    /// too. The versions of one struct are all compiled in - the old ones are
    /// what the migration steps take as their argument - and they are one
    /// line, so each finds the line's record and each would replace it. Whichever the linker happened to hand over last would then
    /// be what the store says it holds, and for a migrated prefix that is the
    /// version it came *from*.
    pub fn ensure_snapshots(&self, held: &HeldBack) -> StorageResult<()> {
        self.provider.atomic(|storage| {
            for entry in crate::schema::declarations() {
                let prefix = &entry.prefix;

                if held.covers(entry) {
                    continue;
                }

                let recording = || {
                    format!(
                        "recording the places {} v{} declares at {prefix}",
                        entry.struct_name, entry.version
                    )
                };

                let mut recorded = storage
                    .get_schema_snapshots(prefix)
                    .attach_with(recording)?;

                let holds = SchemaSnapshot {
                    version: entry.version,
                    id: entry.id.map(str::to_string),
                    struct_name: Some(entry.struct_name.to_string()),
                    fields: entry.fields.iter().map(StoredFieldEntry::from).collect(),
                };

                match moved::record_of(&recorded, entry.id) {
                    Some(at)
                        if recorded[at].version == holds.version
                            && recorded[at].fields == holds.fields =>
                    {
                        continue;
                    }
                    Some(at) if recorded[at].version > holds.version => continue,
                    Some(at) => recorded[at] = holds,
                    None => recorded.push(holds),
                }

                storage
                    .set_schema_snapshots(prefix, &recorded)
                    .attach_with(recording)?;
            }
            Ok(())
        })
    }

    /// Migrates every prefix the code knows about, each with whatever it
    /// reaches into.
    ///
    /// Nothing here decides an order up front. A prefix is migrated when it
    /// comes up, and a step that reaches into another prefix has that one
    /// migrated on the spot, inside this same transaction - so what a reach
    /// reads is the migrated value, and the ordering is the reaching rather
    /// than a list somebody kept in step with it.
    ///
    /// One transaction per prefix a pass starts at, holding it and everything
    /// it reached. A failure rolls that back and leaves the rest of the store
    /// alone, which is what makes one prefix's bad step something the report
    /// can name rather than something that stops the open.
    pub fn run(&self, mset: MigrationSet) -> StorageResult<MigrationReport> {
        if let Some((one, other)) = crate::schema::declared_twice() {
            let mut between = [one.struct_name, other.struct_name];
            between.sort_unstable();

            return Err(Report::new(MigrationError::DeclaredTwice {
                lineage: one.lineage().to_string(),
                version: one.version,
                between: (between[0], between[1]),
            })
            .change_context(StorageError::Migrate));
        }

        if let Some((one, other, at)) = crate::schema::claimed_twice() {
            let mut between = [one.struct_name, other.struct_name];
            between.sort_unstable();

            return Err(Report::new(MigrationError::ClaimedTwice {
                at: at.to_string(),
                between: (between[0], between[1]),
            })
            .change_context(StorageError::Migrate));
        }

        for lineage in mset.known_lineages() {
            let declared = crate::schema::declarations_of(&lineage)
                .map(|entry| entry.version)
                .max();
            let planned = mset
                .get_migration_plan(&lineage)
                .and_then(|plan| plan.steps.iter().map(|step| step.target_version()).max());

            if let (Some(declared), Some(planned)) = (declared, planned)
                && planned > declared
            {
                return Err(Report::new(MigrationError::PlanPastDeclared {
                    lineage: lineage.to_string(),
                    planned,
                    declared,
                })
                .change_context(StorageError::Migrate));
            }
        }

        let mut report = MigrationReport::default();
        let mut done: HashSet<Lineage> = HashSet::new();

        for lineage in mset.known_lineages() {
            if done.contains(&lineage) {
                continue;
            }

            let pass = Pass::new(self, &mset, &done);

            let outcome_res = self.provider.atomic(|storage| {
                if pass.version_is_lost(storage, &lineage.prefix)? {
                    let (target_v, _) = mset.get_target(&lineage);
                    self.record_version(storage, &lineage, target_v)?;

                    return Ok((
                        ComponentOutcome::Skipped(NotMigrated::BookkeepingLost {
                            taken_as: target_v,
                        }),
                        Vec::new(),
                    ));
                }

                if !pass.needs_work(storage, &lineage)? {
                    return Ok((ComponentOutcome::Skipped(NotMigrated::UpToDate), Vec::new()));
                }

                pass.bring_up_to_date(storage, &lineage)?;
                Ok((
                    ComponentOutcome::Committed {
                        steps: pass.steps(),
                    },
                    pass.nagging(),
                ))
            });

            let covered = pass.covered();
            done.extend(covered.iter().cloned());

            let mut named: Vec<StorePath> = covered.iter().map(|one| one.prefix.clone()).collect();
            if named.is_empty() {
                named.push(lineage.prefix.clone());
            }

            match outcome_res {
                Ok((outcome, nagging)) => {
                    report.components.push(ComponentResult {
                        prefixes: named,
                        outcome,
                        nagging,
                    });
                }
                Err(e) => {
                    report.components.push(ComponentResult {
                        prefixes: named,
                        outcome: ComponentOutcome::Failed { error: e },
                        nagging: Vec::new(),
                    });
                }
            }
        }

        report
            .components
            .extend(self.drift_where_no_step_runs(&mset)?);

        let held = HeldBack {
            prefixes: report
                .components
                .iter()
                .filter(|c| matches!(c.outcome, ComponentOutcome::Failed { .. }))
                .flat_map(|c| c.prefixes.iter().cloned())
                .collect(),

            declarations: report
                .components
                .iter()
                .flat_map(|c| c.nagging.iter())
                .map(|nag| mset.get_target(&nag.lineage()).1)
                .collect(),
        };

        self.ensure_snapshots(&held)?;

        Ok(report)
    }

    /// Drift at the prefixes the pass never walked.
    ///
    /// A prefix with steps is looked at inside its own migration, where the
    /// comparison waits until the version says nothing is due - otherwise it
    /// would complain about a shape a step is on its way to fixing. A prefix
    /// with no steps has no such moment: nothing is ever due there, and nothing
    /// is going to fix it, so the comparison is worth making whatever version is
    /// recorded.
    ///
    /// This is what makes drift reach a binary that declares structs and
    /// contains no `#[migrate]` at all, where the pass walks nothing.
    ///
    /// A prefix some step in the binary targets is left alone even when this
    /// set was not given that step. What is wrong there is the open - `build`
    /// where `build_with_migration` was meant - and telling that reader to raise
    /// a version and write a step names the wrong thing; the step is written.
    ///
    /// One transaction for all of it, and a prefix that fails is a `Failed`
    /// component rather than the end of the pass. This is a comparison that
    /// changes nothing, and it should not be able to stop a store opening or
    /// throw away what the prefixes before it found.
    pub(crate) fn drift_where_no_step_runs(
        &self,
        mset: &MigrationSet,
    ) -> StorageResult<Vec<ComponentResult>> {
        let with_steps = mset.known_lineages();
        let compiled = crate::migration::registry::compiled_steps();

        let mut to_look: Vec<(Lineage, &'static [FieldDescriptor])> = Vec::new();

        for entry in crate::schema::declarations() {
            let lineage = entry.lineage();

            if with_steps.contains(&lineage)
                || to_look.iter().any(|(looked, _)| *looked == lineage)
                || compiled
                    .iter()
                    .any(|step| step.prefix.path() == lineage.prefix && step.id == lineage.id())
            {
                continue;
            }

            let (_, declared) = mset.get_target(&lineage);
            to_look.push((lineage, declared));
        }

        if to_look.is_empty() {
            return Ok(Vec::new());
        }

        self.provider.atomic(|storage| {
            let mut found = Vec::new();

            for (lineage, declared) in &to_look {
                match self.drift_at(storage, lineage, declared) {
                    Ok(None) => {}
                    Ok(Some(record)) => found.push(ComponentResult {
                        prefixes: vec![lineage.prefix.clone()],
                        outcome: ComponentOutcome::Skipped(NotMigrated::UpToDate),
                        nagging: vec![record],
                    }),
                    Err(error) => found.push(ComponentResult {
                        prefixes: vec![lineage.prefix.clone()],
                        outcome: ComponentOutcome::Failed { error },
                        nagging: Vec::new(),
                    }),
                }
            }

            Ok(found)
        })
    }

    /// What the places of `lineage` say, for a line no step will touch.
    ///
    /// See [`retire_what_the_steps_answered_for`] for the other half of the
    /// story a record tells.
    fn drift_at(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        lineage: &Lineage,
        declared: &[FieldDescriptor],
    ) -> StorageResult<Option<NaggingRecord>> {
        let moved = self.places_that_moved(storage, lineage, declared)?;

        if !moved.iter().any(|one| one.verdict() == Verdict::Breaks) {
            return Ok(None);
        }

        let diff = self.calculate_drift(storage, lineage, declared)?;

        Ok(Some(NaggingRecord {
            prefix: lineage.prefix.clone(),
            id: lineage.id.clone(),
            diff,
            moved,
        }))
    }

    /// Writes `version` for `lineage`, reading what the prefix holds first so
    /// another line's version - which a step reaching into it may have just
    /// moved - is kept rather than written back over.
    fn record_version(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        lineage: &Lineage,
        version: u32,
    ) -> StorageResult<()> {
        let mut meta = storage.get_meta(&lineage.prefix)?.unwrap_or_default();
        meta.set_version(lineage.id(), version);
        storage.set_meta(&lineage.prefix, &meta)
    }

    /// Where the declared places sit now against where they sat when this
    /// prefix was last written.
    ///
    /// Empty when nothing was written before: a store being opened for the
    /// first time has nothing to have moved from.
    ///
    /// Only this line is compared against its own record, found by its `id`.
    /// Every other line at the prefix has a look of its own - a pass when it
    /// has steps, [`drift_where_no_step_runs`](Self::drift_where_no_step_runs)
    /// when it has none - so comparing it here too would report its drift twice
    /// and hold back this line's record for a move this line did not make.
    ///
    /// A record of a line nothing at the prefix declares in any version released
    /// every place it held, which is what a struct deleted, or given another
    /// `id`, looks like from here. Such a record has no line of its own to be
    /// looked at by, so the first line standing at the prefix answers for it,
    /// and it is said once.
    fn places_that_moved(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        lineage: &Lineage,
        current_fields: &[FieldDescriptor],
    ) -> StorageResult<Vec<Moved>> {
        let at = &lineage.prefix;
        let recorded = storage.get_schema_snapshots(at)?;

        if recorded
            .iter()
            .filter(|was| was.id.as_deref() == lineage.id())
            .count()
            > 1
        {
            return Err(Report::new(MigrationError::RecordedTwice {
                lineage: lineage.to_string(),
            })
            .change_context(StorageError::Migrate));
        }

        let mut found = Vec::new();

        if let Some(index) = moved::record_of(&recorded, lineage.id()) {
            found.extend(moved::between(&recorded[index].fields, current_fields));
        }

        let answers_for_strays = crate::schema::current_at(at)
            .iter()
            .map(|entry| entry.lineage())
            .min()
            .is_none_or(|first| first == *lineage);

        if answers_for_strays {
            for was in &recorded {
                let declared = was.id.as_deref() == lineage.id()
                    || crate::schema::declarations_at(at)
                        .any(|entry| entry.id == was.id.as_deref());

                if !declared {
                    found.extend(moved::between(&was.fields, &[]));
                }
            }
        }

        Ok(found)
    }

    fn calculate_drift(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        lineage: &Lineage,
        current_fields: &[FieldDescriptor],
    ) -> StorageResult<Option<SchemaDiff>> {
        let recorded = storage.get_schema_snapshots(&lineage.prefix)?;

        let Some(at) = moved::record_of(&recorded, lineage.id()) else {
            return Ok(None);
        };
        let old = recorded[at].clone();

        let mut diff = SchemaDiff {
            added: vec![],
            removed: vec![],
        };

        let mut old_fields: HashMap<StorePath, StoredFieldEntry> = old
            .fields
            .into_iter()
            .map(|f| (f.name.clone(), f))
            .collect();

        for f in current_fields {
            if old_fields.remove(&f.name.path()).is_none() {
                diff.added.push(StoredFieldEntry::from(f));
            }
        }

        diff.removed = old_fields.into_values().collect();

        if diff.added.is_empty() && diff.removed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(diff))
        }
    }

    fn migrate_prefix<P2: StorageProvider>(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        lineage: &Lineage,
        mset: &MigrationSet,
        pass: &Pass<'_, P2>,
    ) -> StorageResult<(Vec<AppliedStep>, Vec<NaggingRecord>)> {
        let (target_v, target_fields) = mset.get_target(lineage);
        let prefix_path = lineage.prefix.clone();

        let recorded_v = storage
            .get_meta(&prefix_path)?
            .and_then(|meta| meta.version_of(lineage.id()));

        let mut version = match recorded_v {
            Some(version) => version,
            None => {
                let start_v = mset
                    .get_migration_plan(lineage)
                    .and_then(|m| m.steps.iter().map(|s| s.target_version()).min())
                    .map(|v| v.saturating_sub(1))
                    .unwrap_or(target_v);

                if start_v == target_v || storage.holds_no_data()? {
                    self.record_version(storage, lineage, target_v)?;
                    target_v
                } else {
                    start_v
                }
            }
        };

        let mut nagging = Vec::new();

        if target_v < version {
            return Err(Report::new(MigrationError::Downgrade {
                prefix: lineage.to_string(),
                db_version: version,
                code_version: target_v,
            })
            .change_context(StorageError::Migrate));
        }

        if target_v == version {
            let moved = self.places_that_moved(storage, lineage, target_fields)?;

            if moved.iter().any(|one| one.verdict() == Verdict::Breaks) {
                let diff = self.calculate_drift(storage, lineage, target_fields)?;

                nagging.push(NaggingRecord {
                    prefix: prefix_path.clone(),
                    id: lineage.id.clone(),
                    diff,
                    moved,
                });
            }
        }

        let mut applied_steps = Vec::new();
        if let Some(plan) = mset.get_migration_plan(lineage) {
            applied_steps = self.run_migrator_steps(
                storage,
                lineage,
                plan,
                &mut version,
                target_v,
                mset.provided(),
                pass,
            )?;

            if !applied_steps.is_empty() {
                self.record_version(storage, lineage, version)?;

                let mut history = storage.get_migration_log(&prefix_path)?.unwrap_or_default();
                history.extend(applied_steps.iter().cloned());
                storage.set_migration_log(&prefix_path, &history)?;
            }
        }

        let may_record = nagging.is_empty() || !applied_steps.is_empty();

        if version == target_v && !target_fields.is_empty() && may_record {
            let holds = SchemaSnapshot {
                version: target_v,
                id: lineage.id.clone(),
                struct_name: crate::schema::declarations_of(lineage)
                    .find(|entry| std::ptr::eq(entry.fields, target_fields))
                    .map(|entry| entry.struct_name.to_string()),
                fields: target_fields.iter().map(StoredFieldEntry::from).collect(),
            };

            let mut recorded = storage.get_schema_snapshots(&prefix_path)?;

            if !applied_steps.is_empty() {
                retire_what_the_steps_answered_for(&mut recorded, lineage, target_v);
            }

            match moved::record_of(&recorded, lineage.id()) {
                Some(at) => recorded[at] = holds,
                None => recorded.push(holds),
            }

            if let Some(said) = moved::contradiction(&recorded) {
                return Err(Report::new(MigrationError::Contradiction {
                    prefix: lineage.to_string(),
                    said: said.to_string(),
                })
                .change_context(StorageError::Migrate));
            }

            storage.set_schema_snapshots(&prefix_path, &recorded)?;
        }

        Ok((applied_steps, nagging))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_migrator_steps<P2: StorageProvider>(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        lineage: &Lineage,
        migrator: &MigrationPlan,
        version: &mut u32,
        target_v: u32,
        provided: &crate::migration::provided::Provided,
        pass: &Pass<'_, P2>,
    ) -> StorageResult<Vec<AppliedStep>> {
        if let Some(pair) = migrator
            .steps
            .windows(2)
            .find(|pair| pair[0].target_version() == pair[1].target_version())
        {
            let mut named = [
                pair[0].description().unwrap_or("a step").to_string(),
                pair[1].description().unwrap_or("a step").to_string(),
            ];
            named.sort_unstable();

            return Err(Report::new(MigrationError::StepTwice {
                lineage: lineage.to_string(),
                version: pair[0].target_version(),
                between: (named[0].clone(), named[1].clone()),
            })
            .change_context(StorageError::Migrate));
        }

        let mut new_steps = Vec::new();
        let mut ctx = MigrationContext::new(lineage.prefix.clone(), storage)
            .with_provided(provided)
            .with_reaching(pass);

        for step in &migrator.steps {
            let sv = step.target_version();
            if sv <= *version {
                continue;
            }
            if sv > target_v {
                break;
            }

            if sv != *version + 1 {
                return Err(Report::new(MigrationError::Gap {
                    prefix: lineage.to_string(),
                    reached_version: *version,
                    expected_version: *version + 1,
                })
                .change_context(StorageError::Migrate));
            }

            if let Err(why) = step.run(&mut ctx) {
                let mut report = Report::<StorageError>::from(why);
                if !report.contains::<crate::store::facts::Migrating>() {
                    report = report.attach(crate::store::facts::Migrating(lineage.to_string()));
                }
                return Err(report.attach(format!("taking it from v{} to v{sv}", *version)));
            }

            let applied = AppliedStep {
                prefix: lineage.prefix.to_string(),
                id: lineage.id.clone(),
                target_version: sv,
                description: step.description().map(|s| s.to_string()),
                applied_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            };

            *version = sv;
            new_steps.push(applied);
        }

        Ok(new_steps)
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::migration::meta::PrefixMeta;
    use amethystate_core::path::StorePath;

    fn p(name: &str) -> StorePath {
        StorePath::from_segments([name])
    }

    use crate::migration::context::{decode, encode};
    use crate::migration::fields::FieldDescriptor;
    use crate::migration::meta::StoredShape;
    use crate::store::{CodecFormat, StorageError};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ops::Deref;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tracing_test::traced_test;

    const EMPTY_FIELDS: &[FieldDescriptor] = &[];

    #[derive(Default, Clone)]
    struct InMemoryStorage {
        data: HashMap<StorePath, Vec<u8>>,
        meta: HashMap<StorePath, PrefixMeta>,
        snapshots: HashMap<StorePath, Vec<SchemaSnapshot>>,
        logs: HashMap<StorePath, Vec<AppliedStep>>,
        refuse_logs: bool,
    }

    impl InMemoryStorage {
        fn get_decoded<T: serde::de::DeserializeOwned>(&self, key: &StorePath) -> Option<T> {
            self.data.get(key).map(|b| decode(self, b).unwrap())
        }

        fn holding_data() -> Self {
            let mut storage = Self::default();
            storage.data.insert(p("elsewhere"), b"1".to_vec());
            storage
        }
    }

    impl MigrationBackendAdapter for InMemoryStorage {
        fn format(&self) -> CodecFormat {
            CodecFormat::Json
        }

        fn get(&self, key: &StorePath) -> StorageResult<Option<Vec<u8>>> {
            Ok(self.data.get(key).cloned())
        }
        fn set(&mut self, key: &StorePath, value: &[u8]) -> StorageResult<()> {
            self.data.insert(key.clone(), value.to_vec());
            Ok(())
        }
        fn delete(&mut self, key: &StorePath) -> StorageResult<()> {
            self.data.remove(key);
            Ok(())
        }
        fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
            let mut res = Vec::new();
            for (k, v) in &self.data {
                if k.starts_with(prefix) {
                    res.push((k.clone(), v.clone()));
                }
            }
            Ok(res)
        }
        fn get_meta(&self, prefix: &StorePath) -> StorageResult<Option<PrefixMeta>> {
            Ok(self.meta.get(prefix).cloned())
        }
        fn set_meta(&mut self, prefix: &StorePath, meta: &PrefixMeta) -> StorageResult<()> {
            self.meta.insert(prefix.clone(), meta.clone());
            Ok(())
        }
        fn get_schema_snapshots(&self, prefix: &StorePath) -> StorageResult<Vec<SchemaSnapshot>> {
            Ok(self.snapshots.get(prefix).cloned().unwrap_or_default())
        }
        fn set_schema_snapshots(
            &mut self,
            prefix: &StorePath,
            trees: &[SchemaSnapshot],
        ) -> StorageResult<()> {
            self.snapshots.insert(prefix.clone(), trees.to_vec());
            Ok(())
        }
        fn get_migration_log(&self, prefix: &StorePath) -> StorageResult<Option<Vec<AppliedStep>>> {
            if self.refuse_logs {
                return Err(Report::new(StorageError::Meta).attach("the log would not read"));
            }
            Ok(self.logs.get(prefix).cloned())
        }
        fn set_migration_log(
            &mut self,
            prefix: &StorePath,
            log: &[AppliedStep],
        ) -> StorageResult<()> {
            self.logs.insert(prefix.clone(), log.to_vec());
            Ok(())
        }
    }

    impl StorageProvider for RefCell<InMemoryStorage> {
        fn atomic<F, T>(&self, f: F) -> StorageResult<T>
        where
            F: FnOnce(&mut dyn MigrationBackendAdapter) -> StorageResult<T>,
        {
            let backup = self.borrow().clone();

            let res = {
                let mut guard = self.borrow_mut();
                f(&mut *guard)
            };

            match res {
                Ok(t) => Ok(t),
                Err(e) => {
                    *self.borrow_mut() = backup;
                    Err(e)
                }
            }
        }
    }

    #[test]
    fn a_bookkeeping_failure_names_the_prefix_it_was_migrating() {
        let storage = RefCell::new(InMemoryStorage {
            refuse_logs: true,
            ..InMemoryStorage::holding_data()
        });
        let mset = MigrationSet::default().add(
            p("ledger"),
            MigrationPlan::new().step(1, "init", |_| Ok(())),
            EMPTY_FIELDS,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        let failed = report
            .components
            .iter()
            .find_map(|one| match &one.outcome {
                ComponentOutcome::Failed { error } => Some(format!("{error:?}")),
                _ => None,
            })
            .expect("the log would not read");

        assert!(failed.contains("migrating: ledger"), "{failed}");
    }

    #[test]
    fn test_first_initialization() {
        let storage = RefCell::new(InMemoryStorage::default());
        let mset = MigrationSet::default().add(
            p("ui"),
            MigrationPlan::new().step(1, "init", |_| Ok(())),
            EMPTY_FIELDS,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        assert!(!report.has_failures());
        let meta = storage.borrow().get_meta(&p("ui")).unwrap().unwrap();
        assert_eq!(meta.version_of(None), Some(1));
    }

    #[test]
    fn test_missing_migration_step_does_not_advance_meta() {
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("app"), &PrefixMeta::at(1))
            .unwrap();

        let mset = MigrationSet::default().add(
            p("app"),
            MigrationPlan::new().step(3, "v3", |_| Ok(())),
            EMPTY_FIELDS,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        let ComponentOutcome::Failed { error } = &report.components[0].outcome else {
            panic!("Expected failed migration component");
        };

        assert_eq!(error.current_context(), &StorageError::Migrate);
        let Some(MigrationError::Gap {
            prefix,
            reached_version,
            expected_version,
        }) = error.downcast_ref::<MigrationError>()
        else {
            panic!("Expected migration gap, got {error:?}");
        };

        assert_eq!(prefix, "app");
        assert_eq!(*reached_version, 1);
        assert_eq!(*expected_version, 2);

        let meta = storage.borrow().get_meta(&p("app")).unwrap().unwrap();
        assert_eq!(meta.version_of(None), Some(1));
    }

    #[test]
    fn test_downgrade_error() {
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("app"), &PrefixMeta::at(5))
            .unwrap();

        let mset = MigrationSet::default().add(
            p("app"),
            MigrationPlan::new().step(4, "v4", |_| Ok(())),
            EMPTY_FIELDS,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        let ComponentOutcome::Failed { error } = &report.components[0].outcome else {
            panic!("Expected failed migration component");
        };

        if let Some(MigrationError::Downgrade {
            db_version,
            code_version,
            ..
        }) = error.downcast_ref::<MigrationError>()
        {
            assert_eq!(*db_version, 5);
            assert_eq!(*code_version, 4);
        } else {
            panic!("Expected Downgrade error, got {error:?}");
        }
    }

    #[test]
    fn test_independent_components_success() {
        let storage = RefCell::new(InMemoryStorage::holding_data());
        let mset = MigrationSet::default()
            .add(
                p("a"),
                MigrationPlan::new().step(1, "ok", |ctx| ctx.set("v", &1)),
                EMPTY_FIELDS,
            )
            .add(
                p("b"),
                MigrationPlan::new().step(1, "fail", |_| {
                    Err(MigrationError::Custom("err".into()).into())
                }),
                EMPTY_FIELDS,
            );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        assert!(report.has_failures());
        assert_eq!(
            storage
                .borrow()
                .get_decoded::<i32>(&StorePath::parse_joined("a.v").unwrap())
                .unwrap(),
            1
        );
    }

    #[test]
    fn test_idle_migration_skipped() {
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("app"), &PrefixMeta::at(1))
            .unwrap();
        let val = encode(storage.borrow().deref(), &1).unwrap();

        storage
            .borrow_mut()
            .data
            .insert(StorePath::parse_joined("app.v").unwrap(), val);

        let mset = MigrationSet::default().add(
            p("app"),
            MigrationPlan::new().step(1, "init", |_| Ok(())),
            EMPTY_FIELDS,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        assert!(matches!(
            report.components[0].outcome,
            ComponentOutcome::Skipped(NotMigrated::UpToDate)
        ));
    }

    #[test]
    fn test_partial_migration_within_component() {
        let storage = RefCell::new(InMemoryStorage::holding_data());
        storage
            .borrow_mut()
            .set_meta(&p("a"), &PrefixMeta::at(1))
            .unwrap();

        let a_calls = Arc::new(AtomicUsize::new(0));
        let b_calls = Arc::new(AtomicUsize::new(0));

        let a_cap = a_calls.clone();
        let b_cap = b_calls.clone();

        let mset = MigrationSet::default()
            .add(
                p("a"),
                MigrationPlan::new().step(1, "v1", move |_| {
                    a_cap.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }),
                EMPTY_FIELDS,
            )
            .add(
                p("b"),
                MigrationPlan::new().step(1, "v1", move |_| {
                    b_cap.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }),
                EMPTY_FIELDS,
            );

        let engine = MigrationEngine::new(&storage);
        engine.run(mset).unwrap();

        assert_eq!(a_calls.load(Ordering::SeqCst), 0);
        assert_eq!(b_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_multiple_steps_migration_order() {
        let storage = RefCell::new(InMemoryStorage::holding_data());
        let mset = MigrationSet::default().add(
            p("app"),
            MigrationPlan::new()
                .step(1, "one", |ctx| ctx.set("log", &"1".to_string()))
                .step(2, "two", |ctx| {
                    let mut s: String = ctx.get("log")?.unwrap();
                    s.push('2');
                    ctx.set("log", &s)
                })
                .step(3, "three", |ctx| {
                    let mut s: String = ctx.get("log")?.unwrap();
                    s.push('3');
                    ctx.set("log", &s)
                }),
            EMPTY_FIELDS,
        );

        let engine = MigrationEngine::new(&storage);
        engine.run(mset).unwrap();

        let final_log: String = storage
            .borrow()
            .get_decoded(&StorePath::parse_joined("app.log").unwrap())
            .unwrap();
        assert_eq!(final_log, "123");
    }

    #[test]
    fn test_migration_resume_from_version() {
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("app"), &PrefixMeta::at(1))
            .unwrap();

        let val = encode(storage.borrow().deref(), &"1").unwrap();
        storage
            .borrow_mut()
            .data
            .insert(StorePath::parse_joined("app.log").unwrap(), val);

        let mset = MigrationSet::default().add(
            p("app"),
            MigrationPlan::new()
                .step(1, "init", |_| panic!("Step 1 should be skipped"))
                .step(2, "next", |ctx| {
                    let mut s: String = ctx.get("log")?.unwrap();
                    s.push('2');
                    ctx.set("log", &s)
                }),
            EMPTY_FIELDS,
        );

        let engine = MigrationEngine::new(&storage);
        engine.run(mset).unwrap();

        let final_log: String = storage
            .borrow()
            .get_decoded(&StorePath::parse_joined("app.log").unwrap())
            .unwrap();
        assert_eq!(final_log, "12");
    }

    #[test]
    fn a_record_at_or_past_the_version_arrived_at_is_not_retired() {
        let record = |version| SchemaSnapshot {
            version,
            fields: vec![StoredFieldEntry {
                name: StorePath::segment("gone"),
                type_name: "u32".to_string(),
                shape: StoredShape::field(),
            }],
            id: None,
            struct_name: Some("Gone".to_string()),
        };
        let mut recorded = vec![record(1), record(2), record(5)];

        retire_what_the_steps_answered_for(&mut recorded, &p("nothing_declares_this").into(), 2);

        let kept: Vec<u32> = recorded.iter().map(|one| one.version).collect();
        assert_eq!(kept, vec![2, 5]);
    }

    #[test]
    fn a_field_added_beside_the_others_is_not_drift() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("profile");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta::at(1))
            .unwrap();
        storage
            .borrow_mut()
            .set_schema_snapshots(
                prefix,
                &[SchemaSnapshot {
                    version: 1,
                    fields: vec![StoredFieldEntry {
                        name: StorePath::segment("name"),
                        type_name: "String".to_string(),
                        shape: StoredShape::field(),
                    }],
                    id: None,
                    struct_name: None,
                }],
            )
            .unwrap();

        static CURRENT_FIELDS: &[FieldDescriptor] = &[
            FieldDescriptor::leaf(&["name"], "name", "String"),
            FieldDescriptor::leaf(&["age"], "age", "u32"),
        ];
        let current_fields = CURRENT_FIELDS;

        let mset = MigrationSet::default().add(
            prefix.clone(),
            MigrationPlan::new().step(1, "v1", |_| Ok(())),
            current_fields,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        assert!(
            !report.has_drift(),
            "`age` takes a place nothing declared before, and nothing that was \
             written has moved out from under anything"
        );
    }

    #[test]
    fn a_field_no_longer_declared_is_drift() {
        use crate::store::moved::What;

        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("profile");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta::at(1))
            .unwrap();
        storage
            .borrow_mut()
            .set_schema_snapshots(
                prefix,
                &[SchemaSnapshot {
                    version: 1,
                    fields: vec![
                        StoredFieldEntry {
                            name: StorePath::segment("name"),
                            type_name: "String".to_string(),
                            shape: StoredShape::field(),
                        },
                        StoredFieldEntry {
                            name: StorePath::segment("nickname"),
                            type_name: "String".to_string(),
                            shape: StoredShape::field(),
                        },
                    ],
                    id: None,
                    struct_name: None,
                }],
            )
            .unwrap();

        static CURRENT_FIELDS: &[FieldDescriptor] =
            &[FieldDescriptor::leaf(&["name"], "name", "String")];

        let mset = MigrationSet::default().add(
            prefix.clone(),
            MigrationPlan::new().step(1, "v1", |_| Ok(())),
            CURRENT_FIELDS,
        );

        let report = MigrationEngine::new(&storage).run(mset).unwrap();

        assert!(report.has_drift());

        let moved = &report.components[0].nagging[0].moved;
        let released: Vec<String> = moved
            .iter()
            .filter(|one| one.what == What::Released)
            .map(|one| one.at.to_string())
            .collect();

        assert_eq!(released, ["nickname"], "and it names the place: {moved:?}");
        assert!(moved.iter().any(|one| one.verdict() == Verdict::Breaks));
    }

    #[test]
    fn a_shape_that_changed_without_its_version_is_drift_rather_than_a_quiet_replacement() {
        use crate::store::moved::What;

        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("panel");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta::at(2))
            .unwrap();
        storage
            .borrow_mut()
            .set_schema_snapshots(
                prefix,
                &[SchemaSnapshot {
                    version: 2,
                    fields: vec![
                        StoredFieldEntry {
                            name: StorePath::segment("width"),
                            type_name: "u32".to_string(),
                            shape: StoredShape::field(),
                        },
                        StoredFieldEntry {
                            name: StorePath::segment("height"),
                            type_name: "u32".to_string(),
                            shape: StoredShape::field(),
                        },
                    ],
                    id: None,
                    struct_name: None,
                }],
            )
            .unwrap();

        static CURRENT_FIELDS: &[FieldDescriptor] = &[
            FieldDescriptor::leaf(&["width"], "width", "u32"),
            FieldDescriptor::leaf(&["depth"], "depth", "u32"),
        ];

        let mset = MigrationSet::default().add(
            prefix.clone(),
            MigrationPlan::new().step(2, "v2", |_| Ok(())),
            CURRENT_FIELDS,
        );

        let report = MigrationEngine::new(&storage).run(mset).unwrap();

        assert!(
            report.has_drift(),
            "the shape recorded at version 2 is not the shape declared at version 2"
        );

        let moved = &report.components[0].nagging[0].moved;
        let named: Vec<(String, What)> = moved
            .iter()
            .map(|one| (one.at.to_string(), one.what.clone()))
            .collect();

        assert!(
            named.contains(&("height".to_string(), What::Released)),
            "{named:?}"
        );
        assert!(
            named.contains(&("depth".to_string(), What::Taken)),
            "{named:?}"
        );

        let held = storage.borrow().get_schema_snapshots(prefix).unwrap();
        assert_eq!(
            held.len(),
            1,
            "the recorded shape stands where it stood rather than beside itself"
        );
        assert_eq!(
            held[0]
                .fields
                .iter()
                .map(|one| one.name.to_string())
                .collect::<Vec<_>>(),
            ["width", "height"],
            "a shape that drifted was written over without the version moving"
        );
    }

    #[test]
    fn a_type_that_changed_under_one_name_is_the_readers_business() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("settings");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta::at(1))
            .unwrap();
        storage
            .borrow_mut()
            .set_schema_snapshots(
                prefix,
                &[SchemaSnapshot {
                    version: 1,
                    fields: vec![StoredFieldEntry {
                        name: StorePath::segment("port"),
                        type_name: "u16".to_string(),
                        shape: StoredShape::field(),
                    }],
                    id: None,
                    struct_name: None,
                }],
            )
            .unwrap();

        static CURRENT_FIELDS: &[FieldDescriptor] =
            &[FieldDescriptor::leaf(&["port"], "port", "u32")];
        let current_fields = CURRENT_FIELDS;

        let mset = MigrationSet::default().add(
            prefix.clone(),
            MigrationPlan::new().step(1, "v1", |_| Ok(())),
            current_fields,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        assert!(
            !report.has_drift(),
            "`port` is where it was; that it holds a different type is answered \
             where it is read, not here"
        );
    }

    #[test]
    fn test_drift_nagging_persists_until_migration() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("app");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta::at(1))
            .unwrap();
        storage
            .borrow_mut()
            .set_schema_snapshots(
                prefix,
                &[SchemaSnapshot {
                    version: 1,
                    fields: vec![StoredFieldEntry {
                        name: StorePath::segment("old"),
                        type_name: "i32".to_string(),
                        shape: StoredShape::field(),
                    }],
                    id: None,
                    struct_name: None,
                }],
            )
            .unwrap();

        static NEW_FIELDS: &[FieldDescriptor] = &[FieldDescriptor::leaf(&["new"], "new", "i32")];
        let fields = NEW_FIELDS;

        {
            let mset = MigrationSet::default().add(
                prefix.clone(),
                MigrationPlan::new().step(1, "v1", |_| Ok(())),
                fields,
            );
            let engine = MigrationEngine::new(&storage);
            let report = engine.run(mset).unwrap();
            assert!(report.has_drift(), "Should nag on the first run");
        }

        {
            let mset = MigrationSet::default().add(
                prefix.clone(),
                MigrationPlan::new().step(1, "v1", |_| Ok(())),
                fields,
            );
            let engine = MigrationEngine::new(&storage);
            let report = engine.run(mset).unwrap();
            assert!(
                report.has_drift(),
                "Should STILL nag! The dev hasn't bumped the version!"
            );
        }

        {
            let mset = MigrationSet::default().add(
                prefix.clone(),
                MigrationPlan::new()
                    .step(1, "v1", |_| Ok(()))
                    .step(2, "ack_drift", |_| Ok(())),
                fields,
            );
            let engine = MigrationEngine::new(&storage);
            let report = engine.run(mset).unwrap();

            assert!(!report.has_failures());
            assert!(
                !report.has_drift(),
                "Drift resolved because version was bumped!"
            );
        }

        let meta = storage.borrow().get_meta(prefix).unwrap().unwrap();
        assert_eq!(meta.version_of(None), Some(2));
    }

    #[test]
    fn test_migration_updates_snapshot() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("data");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta::at(1))
            .unwrap();
        storage
            .borrow_mut()
            .set_schema_snapshots(
                prefix,
                &[SchemaSnapshot {
                    version: 1,
                    fields: vec![StoredFieldEntry {
                        name: StorePath::segment("old_f"),
                        type_name: "u8".into(),
                        shape: StoredShape::field(),
                    }],
                    id: None,
                    struct_name: None,
                }],
            )
            .unwrap();

        static V2_FIELDS: &[FieldDescriptor] = &[FieldDescriptor::leaf(&["new_f"], "new_f", "u16")];
        let v2_fields = V2_FIELDS;

        let mset = MigrationSet::default().add(
            prefix.clone(),
            MigrationPlan::new().step(2, "v2", |ctx| ctx.set("new_f", &10u16)),
            v2_fields,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        assert!(
            report.components[0].nagging.is_empty(),
            "Nagging must remain empty during active upgrades"
        );

        let recorded = storage.borrow().get_schema_snapshots(prefix).unwrap();

        let snap = recorded
            .iter()
            .find(|it| it.fields.iter().any(|f| f.name.to_string() == "new_f"))
            .expect("the places the step moved to are recorded");
        assert_eq!(snap.version, 2);
        assert_eq!(snap.fields.len(), 1);
        assert_eq!(snap.fields[0].type_name, "u16");

        assert!(
            !recorded
                .iter()
                .any(|it| it.fields.iter().any(|f| f.name.to_string() == "old_f")),
            "a step took this prefix from 1 to 2, which is what answers for what \
             was claimed at 1 - and a record nothing declares any more, left \
             behind, is a complaint that comes back on every open for ever"
        );
    }

    #[test]
    fn a_field_added_beside_the_others_is_recorded() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("profile");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta::at(1))
            .unwrap();
        storage
            .borrow_mut()
            .set_schema_snapshots(
                prefix,
                &[SchemaSnapshot {
                    version: 1,
                    fields: vec![StoredFieldEntry {
                        name: StorePath::segment("name"),
                        type_name: "String".to_string(),
                        shape: StoredShape::field(),
                    }],
                    id: None,
                    struct_name: None,
                }],
            )
            .unwrap();

        static CURRENT_FIELDS: &[FieldDescriptor] = &[
            FieldDescriptor::leaf(&["name"], "name", "String"),
            FieldDescriptor::leaf(&["age"], "age", "u32"),
        ];

        let mset = MigrationSet::default().add(
            prefix.clone(),
            MigrationPlan::new().step(1, "v1", |_| Ok(())),
            CURRENT_FIELDS,
        );

        let report = MigrationEngine::new(&storage).run(mset).unwrap();
        assert!(!report.has_drift());

        let recorded = storage.borrow().get_schema_snapshots(prefix).unwrap();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(
            recorded[0]
                .fields
                .iter()
                .map(|f| f.name.to_string())
                .collect::<Vec<_>>(),
            ["name", "age"]
        );
    }

    #[traced_test]
    #[test]
    fn drift_is_reported_and_the_log_names_the_places() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("app_settings");

        {
            static FIELDS_V1: &[FieldDescriptor] = &[
                FieldDescriptor::leaf(&["port"], "port", "u16"),
                FieldDescriptor::leaf(&["host"], "host", "String"),
            ];
            let fields_v1 = FIELDS_V1;

            let mset = MigrationSet::default().add(
                prefix.clone(),
                MigrationPlan::new().step(1, "v1", |_| Ok(())),
                fields_v1,
            );

            let engine = MigrationEngine::new(&storage);
            let _ = engine.run(mset).unwrap();
        }

        {
            static FIELDS_V2: &[FieldDescriptor] = &[
                FieldDescriptor::leaf(&["port"], "port", "u32"),
                FieldDescriptor::leaf(&["timeout"], "timeout", "Duration"),
            ];
            let fields_v2 = FIELDS_V2;

            let mset = MigrationSet::default().add(
                prefix.clone(),
                MigrationPlan::new().step(1, "v1", |_| Ok(())),
                fields_v2,
            );

            let engine = MigrationEngine::new(&storage);
            let report = engine.run(mset).unwrap();

            assert!(report.has_drift(), "Report should detect drift");

            report.log_to_tracing();

            #[cfg(not(feature = "diagnostics"))]
            {
                assert!(
                    logs_contain("Schema drift detected in prefix 'app_settings'"),
                    "the drift was reported and the log did not name the prefix"
                );
                assert!(
                    logs_contain("+ field 'timeout'"),
                    "the log did not name the place the code added"
                );
                assert!(
                    logs_contain("- field 'host'"),
                    "the log did not name the place the store still holds"
                );
                assert!(
                    !logs_contain("field 'port'"),
                    "port kept its place and only changed type, which is not drift"
                );
            }

            #[cfg(feature = "diagnostics")]
            {
                assert!(
                    logs_contain("amethystate::drift"),
                    "the drift was reported and the log did not carry the diagnostic"
                );

                let shown: String = report
                    .drift()
                    .iter()
                    .map(|one| format!("{one:?}"))
                    .collect();

                assert!(
                    shown.contains("app_settings.host"),
                    "the rendering did not name the place the store still holds:\n{shown}"
                );
                assert!(
                    shown.contains("app_settings.timeout"),
                    "the rendering did not name the place the code added:\n{shown}"
                );
                assert!(
                    !shown.contains("app_settings.port"),
                    "port kept its place and only changed type, which is not drift:\n{shown}"
                );
            }
        }
    }
}
