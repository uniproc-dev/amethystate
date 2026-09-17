use crate::codec::CodecError;
use crate::store::backend::utils;
use crate::store::builder::Backend;
use crate::store::config::StoreConfig;
use crate::store::durable::{Commit, CommitSignal};
use crate::store::error::{StorageError, StorageResult};
use crate::store::facts::{Facts, Key};
use crate::store::screening::Screening;
use crate::store::traits::StoreLayout;
use crate::store::{
    InitState, StoreBackend, StoreCallback, StoreEvent, StoreOp, SubscriptionEntry, SubscriptionId,
    SubscriptionKind,
};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};
use parking_lot::{Mutex, RwLock};
use rmp_serde::Serializer;
use rmp_serde::config::BytesMode;
use std::collections::{BTreeMap, HashSet};
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
#[derive(Clone)]
pub struct MemoryStore {
    inner: Arc<MemoryStoreInner>,
}

struct MemoryStoreInner {
    data: RwLock<BTreeMap<StorePath, Vec<u8>>>,
    initialized: Mutex<HashSet<StorePath>>,
    subscriptions: RwLock<Vec<SubscriptionEntry>>,
    next_sub_id: AtomicU64,
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
    /// An empty store, under the limits and read settings `config` names.
    /// Nothing else in it applies: there is no file to reach or to save.
    pub fn open(config: &StoreConfig) -> Self {
        Self {
            inner: Arc::new(MemoryStoreInner {
                data: RwLock::new(BTreeMap::new()),
                initialized: Mutex::new(HashSet::new()),
                subscriptions: RwLock::new(Vec::new()),
                next_sub_id: AtomicU64::new(1),
                settled: AtomicU64::new(0),
                commits: Arc::new(CommitSignal::default()),
                closed: AtomicBool::new(false),
                parallel_reads: config.parallel_reads,
                budget: Screening::resolve(&config.limits, Backend::Memory),
            }),
        }
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

    fn under<'a>(
        data: &'a BTreeMap<StorePath, Vec<u8>>,
        prefix: &'a StorePath,
    ) -> impl Iterator<Item = (&'a StorePath, &'a Vec<u8>)> {
        data.range(prefix.clone()..)
            .take_while(move |(path, _)| path.starts_with(prefix))
    }
}

impl StoreBackend for MemoryStore {
    fn get_raw(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        self.refuse_if_closed()?;
        Ok(self.inner.data.read().get(path).cloned())
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
            let mut data = self.inner.data.write();
            self.refuse_if_closed()?;
            if data.get(&path) == Some(&bytes) {
                return Ok(());
            }
            let old = data.insert(path.clone(), bytes.clone());
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
            let mut data = self.inner.data.write();
            self.refuse_if_closed()?;
            let Some(old) = data.remove(path) else {
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
            let mut data = self.inner.data.write();
            self.refuse_if_closed()?;
            let going: Vec<StorePath> = Self::under(&data, prefix)
                .map(|(path, _)| path.clone())
                .collect();
            for path in going {
                data.remove(&path);
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
        let data = self.inner.data.read();
        Ok(Self::under(&data, prefix)
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect())
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        self.refuse_if_closed()?;
        let data = self.inner.data.read();
        Ok(Self::under(&data, prefix)
            .map(|(path, _)| path.clone())
            .collect())
    }

    fn parallel_reads(&self) -> bool {
        self.inner.parallel_reads
    }

    fn files_layout(&self) -> Option<StoreLayout> {
        Some(StoreLayout::InMemory)
    }

    fn save_now(&self) -> StorageResult<()> {
        self.refuse_if_closed()
    }

    fn close(&self) -> StorageResult<()> {
        self.inner.closed.store(true, Ordering::Release);
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    fn subscribe(&self, kind: SubscriptionKind, callback: StoreCallback) -> SubscriptionId {
        let id = self.inner.next_sub_id.fetch_add(1, Ordering::Relaxed);
        self.inner
            .subscriptions
            .write()
            .push(SubscriptionEntry { id, kind, callback });
        id
    }

    fn unsubscribe(&self, id: SubscriptionId) {
        self.inner.subscriptions.write().retain(|s| s.id != id);
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
        Ok(self.inner.initialized.lock().contains(namespace))
    }

    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()> {
        self.refuse_if_closed()?;
        let mut initialized = self.inner.initialized.lock();
        match state.is_seeded() {
            true => initialized.insert(namespace.clone()),
            false => initialized.remove(namespace),
        };
        Ok(())
    }
}
