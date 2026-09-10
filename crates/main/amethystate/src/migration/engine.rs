use crate::migration::context::Reaching;
use crate::migration::fields::FieldDescriptor;
use crate::migration::meta::{PrefixMeta, SchemaSnapshot, StoredFieldEntry};
use crate::migration::set::MigrationSet;
use crate::migration::{
    AppliedStep, ComponentOutcome, ComponentResult, NaggingRecord, NotMigrated, SchemaDiff,
};
use crate::schema::SchemaEntry;
use crate::store::MigrationBackendAdapter;
use crate::store::facts::Facts;
use crate::store::moved::{self, Moved, Verdict};
use crate::store::{StorageError, StorageResult};
use crate::{MigrationContext, MigrationError, MigrationPlan, MigrationReport};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

fn group_path(prefix: &str) -> StorageResult<StorePath> {
    StorePath::parse_joined(prefix)
        .change_context(StorageError::Path)
        .attach_raw_key(prefix)
}

pub trait StorageProvider {
    fn atomic<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&mut dyn MigrationBackendAdapter) -> StorageResult<T>;
}

pub struct MigrationEngine<'a, P: StorageProvider> {
    provider: &'a P,
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

    /// Prefixes an earlier pass already committed.
    settled: &'a HashSet<String>,

    covered: RefCell<Vec<String>>,
    running: RefCell<Vec<String>>,
    steps: RefCell<Vec<AppliedStep>>,
    nagging: RefCell<Vec<NaggingRecord>>,
}

impl<'a, P: StorageProvider> Pass<'a, P> {
    fn new(
        engine: &'a MigrationEngine<'a, P>,
        mset: &'a MigrationSet,
        settled: &'a HashSet<String>,
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

    /// Whether `prefix` is at a version or a shape the code no longer agrees
    /// with.
    fn needs_work(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        prefix: &str,
    ) -> StorageResult<bool> {
        let meta = storage.get_meta(&group_path(prefix)?)?;
        let current_v = meta.as_ref().map(|m| m.version).unwrap_or(0);
        let (target_v, target_fields) = self.mset.get_target(prefix);

        Ok(target_v != current_v
            || !self
                .engine
                .places_that_moved(storage, prefix, target_fields)?
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
        prefix: &str,
    ) -> StorageResult<bool> {
        if !storage.bookkeeping_is_lost() {
            return Ok(false);
        }

        let at = group_path(prefix)?;
        Ok(!storage.scan_prefix(&at)?.is_empty())
    }

    fn bring_up_to_date(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        prefix: &str,
    ) -> StorageResult<()> {
        if self.running.borrow().iter().any(|p| p == prefix) {
            let mut chain = self.running.borrow().clone();
            chain.push(prefix.to_string());

            return Err(
                Report::new(MigrationError::Cycle(chain)).change_context(StorageError::Migrate)
            );
        }

        if self.settled.contains(prefix) || self.covered.borrow().iter().any(|p| p == prefix) {
            return Ok(());
        }

        if self.version_is_lost(storage, prefix)? {
            return Err(Report::new(MigrationError::VersionUnknown {
                prefix: prefix.to_string(),
            })
            .change_context(StorageError::Migrate));
        }

        self.covered.borrow_mut().push(prefix.to_string());
        self.running.borrow_mut().push(prefix.to_string());

        let ran = self.engine.migrate_prefix(storage, prefix, self.mset, self);
        self.running.borrow_mut().pop();

        let (steps, nagging) = ran?;
        self.steps.borrow_mut().extend(steps);
        self.nagging.borrow_mut().extend(nagging);

        Ok(())
    }

    fn covered(&self) -> Vec<String> {
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
        full_key: &str,
    ) -> StorageResult<()> {
        let Some(owner) = self.mset.owner_of(full_key)? else {
            return Ok(());
        };

        if owner == *from {
            return Ok(());
        }

        self.bring_up_to_date(storage, &owner.to_string())
            .attach_with(|| format!("reached from {from} into {full_key}"))
    }
}

impl<'a, P: StorageProvider> MigrationEngine<'a, P> {
    pub fn new(provider: &'a P) -> Self {
        Self { provider }
    }

    /// Records the schema the code declares, so a later run can tell what
    /// changed under it.
    ///
    /// Prefixes whose migration failed are skipped: overwriting their snapshot
    /// with the current schema leaves `calculate_drift` nothing to compare
    /// against, and the diagnostic for the one prefix that needs it is gone for
    /// good.
    pub fn ensure_snapshots(&self, failed: &[StorePath]) -> StorageResult<()> {
        self.provider.atomic(|storage| {
            for entry in inventory::iter::<SchemaEntry> {
                let prefix = &entry.prefix;

                if failed.contains(prefix) {
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
                    struct_name: Some(entry.struct_name.to_string()),
                    fields: entry.fields.iter().map(StoredFieldEntry::from).collect(),
                };

                match moved::same_declaration(&recorded, entry.fields) {
                    Some(at)
                        if recorded[at].version == holds.version
                            && recorded[at].fields == holds.fields =>
                    {
                        continue;
                    }
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
        let mut report = MigrationReport::default();
        let mut done: HashSet<String> = HashSet::new();

        for prefix in mset.known_prefixes() {
            if done.contains(&prefix) {
                continue;
            }

            let pass = Pass::new(self, &mset, &done);

            let outcome_res = self.provider.atomic(|storage| {
                if pass.version_is_lost(storage, &prefix)? {
                    return Ok((
                        ComponentOutcome::Skipped(NotMigrated::VersionUnknown),
                        Vec::new(),
                    ));
                }

                if !pass.needs_work(storage, &prefix)? {
                    return Ok((ComponentOutcome::Skipped(NotMigrated::UpToDate), Vec::new()));
                }

                pass.bring_up_to_date(storage, &prefix)?;
                Ok((
                    ComponentOutcome::Committed {
                        steps: pass.steps(),
                    },
                    pass.nagging(),
                ))
            });

            let covered = pass.covered();
            done.extend(covered.iter().cloned());

            let mut named = covered;
            if named.is_empty() {
                named.push(prefix.clone());
            }

            let named = named
                .iter()
                .map(|held| {
                    StorePath::parse_joined(held)
                        .change_context(StorageError::Path)
                        .attach_raw_key(held)
                        .attach("a prefix a migration set was given cannot be read as a path")
                })
                .collect::<StorageResult<Vec<_>>>()?;

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

        let failed: Vec<StorePath> = report
            .components
            .iter()
            .filter(|c| matches!(c.outcome, ComponentOutcome::Failed { .. }))
            .flat_map(|c| c.prefixes.iter().cloned())
            .collect();

        self.ensure_snapshots(&failed)?;

        Ok(report)
    }

    /// Where the declared places sit now against where they sat when this
    /// prefix was last written.
    ///
    /// Empty when nothing was written before: a store being opened for the
    /// first time has nothing to have moved from.
    fn places_that_moved(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        prefix: &str,
        current_fields: &[FieldDescriptor],
    ) -> StorageResult<Vec<Moved>> {
        let at = group_path(prefix)?;
        let recorded = storage.get_schema_snapshots(&at)?;

        let mut declared: Vec<&[FieldDescriptor]> = vec![current_fields];
        declared.extend(
            inventory::iter::<SchemaEntry>
                .into_iter()
                .filter(|entry| entry.prefix == at)
                .map(|entry| entry.fields),
        );

        let mut found = match moved::same_declaration(&recorded, current_fields) {
            Some(index) => moved::between(&recorded[index].fields, current_fields),
            None => Vec::new(),
        };

        for was in &recorded {
            let met = declared
                .iter()
                .any(|now| moved::same_declaration(std::slice::from_ref(was), now).is_some());

            if !met {
                found.extend(moved::between(&was.fields, &[]));
            }
        }

        Ok(found)
    }

    fn calculate_drift(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        prefix: &str,
        current_fields: &[FieldDescriptor],
    ) -> StorageResult<Option<SchemaDiff>> {
        let recorded = storage.get_schema_snapshots(&group_path(prefix)?)?;

        let Some(at) = moved::same_declaration(&recorded, current_fields) else {
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
        prefix: &str,
        mset: &MigrationSet,
        pass: &Pass<'_, P2>,
    ) -> StorageResult<(Vec<AppliedStep>, Vec<NaggingRecord>)> {
        let (target_v, target_fields) = mset.get_target(prefix);
        let prefix_path = group_path(prefix)?;

        let meta_opt = storage.get_meta(&prefix_path)?;

        let mut meta = match meta_opt {
            Some(m) => m,
            None => {
                let start_v = mset
                    .get_migration_plan(prefix)
                    .and_then(|m| m.steps.iter().map(|s| s.target_version()).min())
                    .map(|v| v.saturating_sub(1))
                    .unwrap_or(target_v);

                if start_v == target_v {
                    storage.set_meta(&prefix_path, &PrefixMeta { version: target_v })?;
                    return Ok((vec![], vec![]));
                }

                PrefixMeta { version: start_v }
            }
        };

        let mut nagging = Vec::new();

        if target_v < meta.version {
            return Err(Report::new(MigrationError::Downgrade {
                prefix: prefix.to_string(),
                db_version: meta.version,
                code_version: target_v,
            })
            .change_context(StorageError::Migrate));
        }

        if target_v == meta.version {
            let moved = self.places_that_moved(storage, prefix, target_fields)?;

            if moved.iter().any(|one| one.verdict() == Verdict::Breaks) {
                let diff = self.calculate_drift(storage, prefix, target_fields)?;

                nagging.push(NaggingRecord {
                    prefix: prefix.to_string(),
                    diff,
                    moved,
                });
            }
        }

        let mut applied_steps = Vec::new();
        if let Some(plan) = mset.get_migration_plan(prefix) {
            let mut history = storage.get_migration_log(&prefix_path)?.unwrap_or_default();

            applied_steps = self.run_migrator_steps(
                storage,
                prefix,
                plan,
                &mut meta,
                target_v,
                &mut history,
                mset.provided(),
                pass,
            )?;

            if !applied_steps.is_empty() {
                storage.set_meta(&prefix_path, &meta)?;
                storage.set_migration_log(&prefix_path, &history)?;
            }
        }

        let unanswered = !nagging.is_empty() && applied_steps.is_empty();

        if meta.version == target_v && !target_fields.is_empty() && !unanswered {
            let holds = SchemaSnapshot {
                version: target_v,
                struct_name: inventory::iter::<SchemaEntry>
                    .into_iter()
                    .find(|e| e.prefix == prefix_path)
                    .map(|e| e.struct_name.to_string()),
                fields: target_fields.iter().map(StoredFieldEntry::from).collect(),
            };

            let mut recorded = storage.get_schema_snapshots(&prefix_path)?;

            match moved::same_declaration(&recorded, target_fields) {
                Some(at) => recorded[at] = holds,
                None => recorded.push(holds),
            }

            storage.set_schema_snapshots(&prefix_path, &recorded)?;
        }

        Ok((applied_steps, nagging))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_migrator_steps<P2: StorageProvider>(
        &self,
        storage: &mut dyn MigrationBackendAdapter,
        prefix: &str,
        migrator: &MigrationPlan,
        meta: &mut PrefixMeta,
        target_v: u32,
        history: &mut Vec<AppliedStep>,
        provided: &crate::migration::provided::Provided,
        pass: &Pass<'_, P2>,
    ) -> StorageResult<Vec<AppliedStep>> {
        let mut new_steps = Vec::new();
        let mut ctx = MigrationContext::new(group_path(prefix)?, storage)
            .with_provided(provided)
            .with_reaching(pass);

        for step in &migrator.steps {
            let sv = step.target_version();
            if sv <= meta.version {
                continue;
            }
            if sv > target_v {
                break;
            }

            if sv != meta.version + 1 {
                return Err(Report::new(MigrationError::Gap {
                    prefix: prefix.to_string(),
                    reached_version: meta.version,
                    expected_version: meta.version + 1,
                })
                .change_context(StorageError::Migrate));
            }

            step.run(&mut ctx)?;

            let applied = AppliedStep {
                prefix: prefix.to_string(),
                target_version: sv,
                description: step.description().map(|s| s.to_string()),
                applied_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            };

            meta.version = sv;
            history.push(applied.clone());
            new_steps.push(applied);
        }

        Ok(new_steps)
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
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
    }

    impl InMemoryStorage {
        fn get_decoded<T: serde::de::DeserializeOwned>(&self, key: &StorePath) -> Option<T> {
            self.data.get(key).map(|b| decode(self, b).unwrap())
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
            Ok(self
                .snapshots
                .get(prefix)
                .cloned()
                .unwrap_or_default())
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
    fn test_first_initialization() {
        let storage = RefCell::new(InMemoryStorage::default());
        let mset = MigrationSet::default().add(
            "ui",
            MigrationPlan::new().step(1, "init", |_| Ok(())),
            EMPTY_FIELDS,
        );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        assert!(!report.has_failures());
        let meta = storage.borrow().get_meta(&p("ui")).unwrap().unwrap();
        assert_eq!(meta.version, 1);
    }

    #[test]
    fn test_missing_migration_step_does_not_advance_meta() {
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("app"), &PrefixMeta { version: 1 })
            .unwrap();

        let mset = MigrationSet::default().add(
            "app",
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
        assert_eq!(meta.version, 1);
    }

    #[test]
    fn test_downgrade_error() {
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("app"), &PrefixMeta { version: 5 })
            .unwrap();

        let mset = MigrationSet::default().add(
            "app",
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
        let storage = RefCell::new(InMemoryStorage::default());
        let mset = MigrationSet::default()
            .add(
                "a",
                MigrationPlan::new().step(1, "ok", |ctx| ctx.set("v", &1)),
                EMPTY_FIELDS,
            )
            .add(
                "b",
                MigrationPlan::new().step(1, "fail", |_| {
                    Err(MigrationError::Custom("err".into()).into())
                }),
                EMPTY_FIELDS,
            );

        let engine = MigrationEngine::new(&storage);
        let report = engine.run(mset).unwrap();

        assert!(report.has_failures());
        assert_eq!(storage.borrow().get_decoded::<i32>(&StorePath::parse_joined("a.v").unwrap()).unwrap(), 1);
    }

    #[test]
    fn test_idle_migration_skipped() {
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("app"), &PrefixMeta { version: 1 })
            .unwrap();
        let val = encode(storage.borrow().deref(), &1).unwrap();

        storage
            .borrow_mut()
            .data
            .insert(StorePath::parse_joined("app.v").unwrap(), val);

        let mset = MigrationSet::default().add(
            "app",
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
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("a"), &PrefixMeta { version: 1 })
            .unwrap();

        let a_calls = Arc::new(AtomicUsize::new(0));
        let b_calls = Arc::new(AtomicUsize::new(0));

        let a_cap = a_calls.clone();
        let b_cap = b_calls.clone();

        let mset = MigrationSet::default()
            .add(
                "a",
                MigrationPlan::new().step(1, "v1", move |_| {
                    a_cap.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }),
                EMPTY_FIELDS,
            )
            .add(
                "b",
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
        let storage = RefCell::new(InMemoryStorage::default());
        let mset = MigrationSet::default().add(
            "app",
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

        let final_log: String = storage.borrow().get_decoded(&StorePath::parse_joined("app.log").unwrap()).unwrap();
        assert_eq!(final_log, "123");
    }

    #[test]
    fn test_migration_resume_from_version() {
        let storage = RefCell::new(InMemoryStorage::default());
        storage
            .borrow_mut()
            .set_meta(&p("app"), &PrefixMeta { version: 1 })
            .unwrap();

        let val = encode(storage.borrow().deref(), &"1").unwrap();
        storage
            .borrow_mut()
            .data
            .insert(StorePath::parse_joined("app.log").unwrap(), val);

        let mset = MigrationSet::default().add(
            "app",
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

        let final_log: String = storage.borrow().get_decoded(&StorePath::parse_joined("app.log").unwrap()).unwrap();
        assert_eq!(final_log, "12");
    }

    #[test]
    fn a_field_added_beside_the_others_is_not_drift() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("profile");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta { version: 1 })
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
            prefix.to_string(),
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
            .set_meta(prefix, &PrefixMeta { version: 1 })
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
                    struct_name: None,
                }],
            )
            .unwrap();

        static CURRENT_FIELDS: &[FieldDescriptor] =
            &[FieldDescriptor::leaf(&["name"], "name", "String")];

        let mset = MigrationSet::default().add(
            prefix.to_string(),
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
    fn a_type_that_changed_under_one_name_is_the_readers_business() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("settings");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta { version: 1 })
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
                    struct_name: None,
                }],
            )
            .unwrap();

        static CURRENT_FIELDS: &[FieldDescriptor] =
            &[FieldDescriptor::leaf(&["port"], "port", "u32")];
        let current_fields = CURRENT_FIELDS;

        let mset = MigrationSet::default().add(
            prefix.to_string(),
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
            .set_meta(prefix, &PrefixMeta { version: 1 })
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
                    struct_name: None,
                }],
            )
            .unwrap();

        static NEW_FIELDS: &[FieldDescriptor] = &[FieldDescriptor::leaf(&["new"], "new", "i32")];
        let fields = NEW_FIELDS;

        {
            let mset = MigrationSet::default().add(
                prefix.to_string(),
                MigrationPlan::new().step(1, "v1", |_| Ok(())),
                fields,
            );
            let engine = MigrationEngine::new(&storage);
            let report = engine.run(mset).unwrap();
            assert!(report.has_drift(), "Should nag on the first run");
        }

        {
            let mset = MigrationSet::default().add(
                prefix.to_string(),
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
                prefix.to_string(),
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
        assert_eq!(meta.version, 2);
    }

    #[test]
    fn test_migration_updates_snapshot() {
        let storage = RefCell::new(InMemoryStorage::default());
        let prefix = &p("data");

        storage
            .borrow_mut()
            .set_meta(prefix, &PrefixMeta { version: 1 })
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
                    struct_name: None,
                }],
            )
            .unwrap();

        static V2_FIELDS: &[FieldDescriptor] = &[FieldDescriptor::leaf(&["new_f"], "new_f", "u16")];
        let v2_fields = V2_FIELDS;

        let mset = MigrationSet::default().add(
            prefix.to_string(),
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
            recorded
                .iter()
                .any(|it| it.fields.iter().any(|f| f.name.to_string() == "old_f")),
            "the tree does not shrink by itself: what claimed old_f is kept \
             until something says to drop it"
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
                prefix.to_string(),
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
                prefix.to_string(),
                MigrationPlan::new().step(1, "v1", |_| Ok(())),
                fields_v2,
            );

            let engine = MigrationEngine::new(&storage);
            let report = engine.run(mset).unwrap();

            assert!(report.has_drift(), "Report should detect drift");

            report.log_to_tracing();

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
    }
}
