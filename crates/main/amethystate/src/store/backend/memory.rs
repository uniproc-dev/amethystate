use crate::MigrationReport;
use crate::codec::CodecError;
use crate::migration::AppliedStep;
use crate::migration::engine::{MigrationEngine, StorageProvider};
use crate::migration::set::MigrationSet;
use crate::store::backend::utils;
use crate::store::builder::Backend;
use crate::store::config::StoreConfig;
use crate::store::durable::{Commit, CommitSignal};
use crate::store::error::{StorageError, StorageResult};
use crate::store::facts::{Facts, Key};
use crate::store::format::{self, StorageFactSet};
use crate::store::meta::{PrefixMeta, SchemaSnapshot};
use crate::store::screening::Screening;
use crate::store::traits::{MigrationBackendAdapter, StoreLayout};
use crate::store::{
    CodecFormat, InitState, StoreBackend, StoreCallback, StoreEvent, StoreOp, SubscriptionEntry,
    SubscriptionId, SubscriptionKind,
};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
use parking_lot::RwLock;
use rmp_serde::Serializer;
use rmp_serde::config::BytesMode;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use uuid::Uuid;

/// A store with no file.
///
/// It keeps what redb keeps - one value per path, written as MessagePack - in
/// a sorted map, and nothing outlives it: there is no buffer, no background
/// thread and nothing to flush, so every commit has landed by the time it is
/// asked for. What it refuses is what redb refuses, which is less than any
/// other engine here, so a store that falls back to it keeps taking every
/// write it took on disk.
///
/// The bookkeeping lives beside the values - the version each line stands at,
/// the schemas recorded, the migration log, the format record, which
/// namespaces were seeded - so a migration runs over it the way it runs over a
/// file.
#[derive(Clone)]
pub struct MemoryStore {
    inner: Arc<MemoryStoreInner>,
}

/// Everything a store holds, values and bookkeeping together, in one piece
/// that a migration can take a copy of and put back.
#[derive(Clone, Default)]
pub(crate) struct Held {
    pub(crate) data: BTreeMap<StorePath, Vec<u8>>,
    pub(crate) meta: BTreeMap<StorePath, PrefixMeta>,
    pub(crate) snapshots: BTreeMap<StorePath, Vec<SchemaSnapshot>>,
    pub(crate) logs: BTreeMap<StorePath, Vec<AppliedStep>>,
    pub(crate) seeded: BTreeSet<StorePath>,
    pub(crate) format: Option<StorageFactSet>,
}

impl Held {
    fn under<'a>(
        &'a self,
        prefix: &'a StorePath,
    ) -> impl Iterator<Item = (&'a StorePath, &'a Vec<u8>)> {
        self.data
            .range(prefix.clone()..)
            .take_while(move |(path, _)| path.starts_with(prefix))
    }
}

impl MigrationBackendAdapter for Held {
    fn format(&self) -> CodecFormat {
        CodecFormat::MessagePack
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
        Ok(self
            .under(prefix)
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect())
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
        Ok(self.logs.get(prefix).cloned())
    }

    fn set_migration_log(&mut self, prefix: &StorePath, log: &[AppliedStep]) -> StorageResult<()> {
        self.logs.insert(prefix.clone(), log.to_vec());
        Ok(())
    }
}

/// Runs a migration pass over what is held, putting the copy taken before it
/// back if the pass fails.
struct HeldProvider<'a>(&'a RwLock<Held>);

impl StorageProvider for HeldProvider<'_> {
    fn atomic<F, T>(&self, f: F) -> StorageResult<T>
    where
        F: FnOnce(&mut dyn MigrationBackendAdapter) -> StorageResult<T>,
    {
        let mut held = self.0.write();
        let before = held.clone();
        let outcome = f(&mut *held);
        if outcome.is_err() {
            *held = before;
        }
        outcome
    }
}

struct MemoryStoreInner {
    held: RwLock<Held>,
    subscriptions: RwLock<Vec<SubscriptionEntry>>,
    settled: AtomicU64,
    commits: Arc<CommitSignal>,
    closed: AtomicBool,
    parallel_reads: bool,
    budget: Screening,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore").finish_non_exhaustive()
    }
}

impl MemoryStore {
    /// An empty store, under the limits and read settings `config` names, with
    /// `migrations` run over it. Nothing else in `config` applies: there is no
    /// file to reach or to save.
    pub fn open(
        config: &StoreConfig,
        migrations: MigrationSet,
    ) -> StorageResult<(Self, MigrationReport)> {
        Self::over(config, Held::default(), migrations)
    }

    /// A store over what `held` already holds, with `migrations` run over it.
    pub(crate) fn over(
        config: &StoreConfig,
        held: Held,
        migrations: MigrationSet,
    ) -> StorageResult<(Self, MigrationReport)> {
        let store = Self {
            inner: Arc::new(MemoryStoreInner {
                held: RwLock::new(held),
                subscriptions: RwLock::new(Vec::new()),
                settled: AtomicU64::new(0),
                commits: Arc::new(CommitSignal::default()),
                closed: AtomicBool::new(false),
                parallel_reads: config.parallel_reads,
                budget: Screening::resolve(&config.limits, Backend::Memory),
            }),
        };

        format::settle(&store, Backend::Memory).attach("opening the store")?;

        let provider = HeldProvider(&store.inner.held);
        let report = MigrationEngine::new(&provider)
            .run(migrations)
            .attach("opening the store")?;

        Ok((store, report))
    }

    fn refuse_if_closed(&self) -> StorageResult<()> {
        match self.inner.closed.load(Ordering::Acquire) {
            true => Err(Report::new(StorageError::Closed).attach("the store lived in memory")),
            false => Ok(()),
        }
    }

    fn encode(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
    ) -> StorageResult<Vec<u8>> {
        let budget = &self.inner.budget;
        budget.check_path(path)?;

        let depth = budget.for_value(path);
        let counted = depth.count(value);
        let mut bytes = Vec::new();
        let mut ser = Serializer::new(&mut bytes)
            .with_bytes(BytesMode::ForceAll)
            .with_struct_map();

        serde::Serialize::serialize(&counted, &mut ser)
            .map_err(CodecError::from)
            .map_err(|why| match depth.overflowed() {
                true => budget.too_deep(path),
                false => Report::new(why)
                    .change_context(StorageError::Codec)
                    .attach(Key(path.clone())),
            })?;

        budget.settle_the_enum_for_a_reader(&depth, value, path);

        match budget.refused(&depth, path) {
            Some(refusal) => Err(refusal),
            None => Ok(bytes),
        }
    }

    fn settle(&self) -> u64 {
        self.inner.settled.fetch_add(1, Ordering::AcqRel) + 1
    }
}

impl StoreBackend for MemoryStore {
    fn get_raw(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        self.refuse_if_closed()?;
        Ok(self.inner.held.read().data.get(path).cloned())
    }

    fn set_erased(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.set_owned_erased(path.clone(), value, source)
    }

    fn set_owned_erased(
        &self,
        path: StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.refuse_if_closed()?;
        let bytes = self.encode(&path, value)?;

        let (old, settled) = {
            let mut held = self.inner.held.write();
            self.refuse_if_closed()?;
            if held.data.get(&path) == Some(&bytes) {
                return Ok(());
            }
            let old = held.data.insert(path.clone(), bytes.clone());
            (old, self.settle())
        };

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path,
                op: StoreOp::Set,
                old,
                new: Some(bytes),
                source: source.into(),
                at: settled,
            },
        )
    }

    fn get_erased(
        &self,
        path: &StorePath,
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<bool> {
        let Some(bytes) = self.get_raw(path)? else {
            return Ok(false);
        };

        self.decode_erased(&bytes, f)
            .change_context(StorageError::Read)
            .attach_key(path)?;
        Ok(true)
    }

    fn decode_erased(
        &self,
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()> {
        let mut de = rmp_serde::Deserializer::from_read_ref(bytes);
        let mut erased = <dyn erased_serde::Deserializer>::erase(&mut de);
        f(&mut erased).attach_value_bytes(bytes.len())
    }

    fn delete_with_source(&self, path: &StorePath, source: Option<Uuid>) -> StorageResult<()> {
        self.refuse_if_closed()?;

        let (old, settled) = {
            let mut held = self.inner.held.write();
            self.refuse_if_closed()?;
            let Some(old) = held.data.remove(path) else {
                return Ok(());
            };
            (old, self.settle())
        };

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path: path.clone(),
                op: StoreOp::Delete,
                old: Some(old),
                new: None,
                source: source.into(),
                at: settled,
            },
        )
    }

    fn delete(&self, path: &StorePath) -> StorageResult<()> {
        self.delete_with_source(path, None)
    }

    fn delete_prefix_with_source(
        &self,
        prefix: &StorePath,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.refuse_if_closed()?;

        let settled = {
            let mut held = self.inner.held.write();
            self.refuse_if_closed()?;
            let going: Vec<StorePath> = held.under(prefix).map(|(path, _)| path.clone()).collect();
            for path in going {
                held.data.remove(&path);
            }
            self.settle()
        };

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path: prefix.clone(),
                op: StoreOp::DeletePrefix,
                old: None,
                new: None,
                source: source.into(),
                at: settled,
            },
        )
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        self.refuse_if_closed()?;
        self.inner.held.read().scan_prefix(prefix)
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        self.refuse_if_closed()?;
        Ok(self
            .inner
            .held
            .read()
            .under(prefix)
            .map(|(path, _)| path.clone())
            .collect())
    }

    fn parallel_reads(&self) -> bool {
        self.inner.parallel_reads
    }

    fn files_layout(&self) -> Option<StoreLayout> {
        Some(StoreLayout::InMemory)
    }

    #[cfg(feature = "test-utils")]
    fn format_record(&self) -> Option<&dyn crate::store::format::TestFormatRecord> {
        Some(self)
    }

    fn save_now(&self) -> StorageResult<()> {
        self.refuse_if_closed()
    }

    fn close(&self) -> StorageResult<()> {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.commits.closed();
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    fn subscribe(&self, kind: SubscriptionKind, callback: StoreCallback) -> SubscriptionId {
        utils::subscribe(&self.inner.subscriptions, kind, callback)
    }

    fn unsubscribe(&self, id: SubscriptionId) -> bool {
        utils::unsubscribe(&self.inner.subscriptions, id)
    }

    fn flush_prefix(&self, _prefix: &StorePath) -> StorageResult<()> {
        self.refuse_if_closed()
    }

    fn flush_async(&self) -> Commit {
        if self.is_closed() {
            return Commit::gone();
        }

        let signal = &self.inner.commits;
        let commit = Commit::awaiting(signal.clone());
        let flush = signal.begin();
        signal.settle(flush, &Ok(()));
        commit
    }

    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool> {
        self.refuse_if_closed()?;
        Ok(self.inner.held.read().seeded.contains(namespace))
    }

    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()> {
        self.refuse_if_closed()?;
        let mut held = self.inner.held.write();
        match state.is_seeded() {
            true => held.seeded.insert(namespace.clone()),
            false => held.seeded.remove(namespace),
        };
        Ok(())
    }

    fn record_schema(&self, at: &StorePath, schema: &SchemaSnapshot) -> StorageResult<()> {
        self.refuse_if_closed()?;
        let mut held = self.inner.held.write();
        let trees = held.snapshots.entry(at.clone()).or_default();
        crate::store::moved::record_into(trees, schema);
        Ok(())
    }
}

impl format::FormatRecord for MemoryStore {
    fn format_facts(&self) -> StorageResult<Option<StorageFactSet>> {
        Ok(self.inner.held.read().format.clone())
    }

    fn set_format_facts(&self, facts: &StorageFactSet) -> StorageResult<()> {
        self.inner.held.write().format = Some(facts.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::ComponentOutcome;
    use crate::migration::MigrationError;
    use crate::migration::builder::MigrationBuilder;
    use std::path::PathBuf;

    fn at(levels: &[&str]) -> StorePath {
        StorePath::from_segments(levels.iter().copied())
    }

    fn holding_port(port: u16) -> Held {
        let mut held = Held::default();
        held.data.insert(
            at(&["net", "port"]),
            rmp_serde::to_vec_named(&port).unwrap(),
        );
        held
    }

    fn read_port(store: &MemoryStore) -> Option<u16> {
        store
            .get_raw(&at(&["net", "port"]))
            .unwrap()
            .map(|bytes| rmp_serde::from_slice(&bytes).unwrap())
    }

    fn steps(configure: impl FnOnce(&mut MigrationBuilder)) -> MigrationSet {
        let mut builder = MigrationBuilder::default();
        configure(&mut builder);
        builder.into_set().unwrap()
    }

    #[test]
    fn a_step_runs_over_what_was_already_held() {
        let (store, report) = MemoryStore::over(
            &StoreConfig::new(PathBuf::new()),
            holding_port(80),
            steps(|m| {
                m.for_prefix("net")
                    .step(1, "move off the privileged port", |ctx| {
                        ctx.set("port", &8080u16)
                    });
            }),
        )
        .unwrap();

        assert!(!report.has_failures(), "{report:?}");
        assert_eq!(read_port(&store), Some(8080));
        assert_eq!(
            store
                .inner
                .held
                .read()
                .meta
                .get(&at(&["net"]))
                .and_then(|meta| meta.version_of(None)),
            Some(1)
        );
    }

    #[test]
    fn a_step_that_fails_leaves_what_was_held_as_it_was() {
        let (store, report) = MemoryStore::over(
            &StoreConfig::new(PathBuf::new()),
            holding_port(80),
            steps(|m| {
                m.for_prefix("net")
                    .step(1, "writes, then turns down", |ctx| {
                        ctx.set("port", &9090u16)?;
                        Err(MigrationError::Custom("this data is not ours".into()).into())
                    });
            }),
        )
        .unwrap();

        assert!(
            report
                .components
                .iter()
                .any(|one| matches!(one.outcome, ComponentOutcome::Failed { .. })),
            "{report:?}"
        );
        assert_eq!(read_port(&store), Some(80));
        assert!(!store.inner.held.read().meta.contains_key(&at(&["net"])));
    }
}
