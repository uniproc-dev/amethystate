use crate::store::{
    InitState, SchemaAwareStore, StoreBackend, StoreCallback, StoreEvent, StoreOp,
    SubscriptionEntry, SubscriptionId, SubscriptionKind,
};
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use migration::RedbMigrationBackend;
use redb::{Database, ReadOnlyTable, ReadableDatabase, TableHandle};
use std::collections::HashSet;
use std::path::Path;
use tables::{TABLE_DATA, TABLE_DIFF_LOG, TABLE_META, TABLE_MIGRATION_LOG};

use crate::store::builder::Backend;
use crate::store::config::StoreConfig;
use crate::store::screening::Screening;
use crate::store::facts::{Facts, Key, StoreFile};
use crate::{
    MigrationReport,
    store::error::{StorageError, StorageResult},
};

use crate::codec::CodecError;
use crate::migration::engine::{MigrationEngine, StorageProvider};
use crate::migration::set::MigrationSet;
use crate::store::backend::redb::tables::TABLE_SCHEMA_SNAPSHOT;
use crate::store::backend::utils;
use crate::store::backend::utils::Attempted;
use crate::store::durable::{Commit, CommitSignal, PersistHealth};
use crate::store::traits::{MigrationBackendAdapter, StoreLayout};
use crate::store::debouncer::Debouncer;
use parking_lot::{Mutex, RwLock};
use rmp_serde::Serializer;
use rmp_serde::config::BytesMode;
use std::cell::RefCell;
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::info;
use uuid::Uuid;

pub mod error;
mod inspector;
mod migration;
mod recovery;
mod tables;

use recovery::{OpenDatabase, create_database, is_previous_io, reopen};

const BUF_SIZE: usize = 64 * 1024;

#[cfg(test)]
static SIMULATE_WRITE_FAILURE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static SERIALIZATION_BUFFER: RefCell<Vec<u8>> =
        RefCell::new(Vec::with_capacity(BUF_SIZE));
}

/// Writes every buffered change into `txn`'s tables. Committing `txn` is the
/// caller's own last step, not this function's - a synchronous flush that
/// reports its error immediately and a retried background one both walk the
/// same changes the same way, and only differ in what happens after this
/// returns.
fn apply_pending(
    txn: &redb::WriteTransaction,
    changes: &utils::Pending,
    path: &Path,
) -> StorageResult<()> {
    let mut table = txn
        .open_table(TABLE_DATA)
        .doing(StorageError::Flush, path)
        .attach_table(TABLE_DATA.name())?;
    let mut meta = txn
        .open_table(TABLE_META)
        .doing(StorageError::Flush, path)
        .attach_table(TABLE_META.name())?;

    for (key, op) in changes {
        match op {
            utils::PendingOp::Set(b) => {
                table
                    .insert(key.as_str(), &b[..])
                    .doing(StorageError::Flush, path)
                    .attach_table(TABLE_DATA.name())
                    .attach_key(key)
                    .attach_value_bytes(b.len())?;
            }
            utils::PendingOp::Delete => {
                table
                    .remove(key.as_str())
                    .doing(StorageError::Flush, path)
                    .attach_table(TABLE_DATA.name())
                    .attach_key(key)?;
            }
            utils::PendingOp::Init(seeded) => {
                let init_key = utils::init_key(key.as_str());
                if *seeded {
                    meta.insert(init_key.as_str(), &[][..]).map(|_| ())
                } else {
                    meta.remove(init_key.as_str()).map(|_| ())
                }
                .doing(StorageError::Flush, path)
                .attach_table(TABLE_META.name())
                .attach_prefix(key)?;
            }
        }
    }

    Ok(())
}

struct RedbStoreInner {
    db: OpenDatabase,
    path: Arc<Path>,
    pending: Arc<Mutex<utils::Pending>>,
    initialized: Arc<Mutex<HashSet<StorePath>>>,
    commits: Arc<CommitSignal>,
    health: Arc<PersistHealth>,
    debouncer: Arc<Debouncer>,
    subscriptions: Arc<RwLock<Vec<SubscriptionEntry>>>,
    next_sub_id: Arc<AtomicU64>,
    write_lock: Arc<Mutex<()>>,
    parallel_reads: bool,
    /// What this store may spend on a path and its value together. redb needs
    /// it most: `rmp_serde` has no limit of its own, so a value deep enough
    /// commits and then kills every process that opens the file afterwards.
    budget: Screening,
}

impl RedbStoreInner {
    /// Writes what is buffered, stops the background thread and lets go of the
    /// file.
    ///
    /// The order is what makes it safe. Stopping first means a write racing
    /// this one is refused rather than buffered by a store about to lose the
    /// handle, and the join means the flush the thread was already running has
    /// finished. Only then is the database dropped, which is what releases the
    /// file to whoever the close was for.
    ///
    /// Closing twice is fine: the second call finds the thread stopped and
    /// returns, so `Drop` after an explicit close does nothing.
    pub fn close(&self) -> StorageResult<()> {
        {
            let _buffering = self.pending.lock();
            if !self.debouncer.stop_accepting() {
                return Ok(());
            }
        }
        info!("Closing RedbStore...");

        self.debouncer.shutdown();
        let flushed = self.save_now().attach("flushing the buffer before close");
        self.db.store(None);

        flushed
    }

    /// Buffers a write, refusing it if the store closed first.
    ///
    /// The refusal belongs inside this lock rather than at the top of a write.
    /// Closing takes the same lock to decide it is closing, so a write is
    /// either in the buffer before that decision - and so in the flush that
    /// follows it - or it is refused. Checked earlier and buffered later, a
    /// write lands after the last flush and is reported as taken while never
    /// reaching the disk.
    fn buffer(&self, fill: impl FnOnce(&mut utils::Pending)) -> StorageResult<()> {
        let mut lock = self.pending.lock();
        if self.debouncer.is_stopped() {
            return Err(error_stack::Report::new(StorageError::Closed)
                .attach(StoreFile(self.path.to_path_buf())));
        }
        fill(&mut lock);
        Ok(())
    }

    pub fn save_now(&self) -> StorageResult<()> {
        self.flush_prefix(&StorePath::root())
    }

    /// Commits what is buffered under `prefix`, trading the handle in if redb
    /// has stopped touching the disk.
    ///
    /// This is the path a durable write waits on, so it recovers rather than
    /// reporting a failure the caller can do nothing about: a handle that has
    /// seen an I/O error answers everything with `PreviousIo` for good, and
    /// only a fresh one can land the write. One retry, because the second
    /// failure is the disk rather than the handle.
    pub fn flush_prefix(&self, prefix: &StorePath) -> StorageResult<()> {
        let _write_guard = self.write_lock.lock();

        match self.flush_locked(prefix) {
            Err(report) if is_previous_io(&report) => {
                reopen(&self.db, &self.path)?;
                self.flush_locked(prefix)
            }
            other => other,
        }
    }

    fn flush_locked(&self, prefix: &StorePath) -> StorageResult<()> {
        let changes = {
            let lock = self.pending.lock();
            utils::pending_prefix(&lock, prefix)
        };
        let txn = self
            .db()?
            .begin_write()
            .doing(StorageError::Flush, &self.path)
            .attach_prefix(prefix)
            .attach_buffered(changes.len())?;

        apply_pending(&txn, &changes, &self.path)?;

        txn.commit()
            .doing(StorageError::Flush, &self.path)
            .attach_prefix(prefix)
            .attach_buffered(changes.len())?;

        utils::clear_committed(&mut self.pending.lock(), &changes);
        self.commits.finished(true);
        Ok(())
    }

    /// The database to work against, or a failure if it is being replaced.
    ///
    /// A read or a scan calling this during the gap is told so rather than
    /// waiting: a reopen is triggered by a disk that already failed, and a UI
    /// thread blocking on a file operation is what this library avoids
    /// everywhere else.
    ///
    /// A durable write does wait, and needs no code here to do it: it goes
    /// through `flush_prefix`, which takes `write_lock` first, and the reopen
    /// holds that same lock for the whole swap. So a commit either runs before
    /// the reopen or after it, and never during - which is the blocking a
    /// durable write already promises. Keep the two on one lock and that stays
    /// true for free.
    fn db(&self) -> StorageResult<Arc<Database>> {
        self.db.load_full().ok_or_else(|| {
            if self.debouncer.is_stopped() {
                error_stack::Report::new(StorageError::Closed)
                    .attach(StoreFile(self.path.to_path_buf()))
            } else {
                error_stack::Report::new(StorageError::Read)
                    .attach("the database is being reopened after an I/O failure")
                    .attach(StoreFile(self.path.to_path_buf()))
            }
        })
    }

    /// A read transaction and the data table in it.
    ///
    /// The transaction comes back with the table because dropping it would end
    /// the read, and `ReadOnlyTable` does not borrow from it - so it has to be
    /// held rather than being kept alive by the borrow checker.
    fn read_data(
        &self,
        what: StorageError,
    ) -> StorageResult<(redb::ReadTransaction, ReadOnlyTable<&'static str, &'static [u8]>)> {
        let txn = self
            .db()?
            .begin_read()
            .doing(what, &self.path)
            .attach_table(TABLE_DATA.name())?;
        let table = txn
            .open_table(TABLE_DATA)
            .doing(what, &self.path)
            .attach_table(TABLE_DATA.name())?;
        Ok((txn, table))
    }

    /// Whether a write may proceed.
    ///
    /// A background flush that has been failing past its budget is an error
    /// the caller can act on, not a reason to take the process down - the
    /// value is refused, what is already buffered keeps being retried, and a
    /// flush that lands clears this. A debouncer thread that is actually dead
    /// is a different thing and still panics: that is a bug here, not a disk.
    fn check_debouncer(&self) -> StorageResult<()> {
        utils::check_debouncer(&self.health, &self.debouncer)
    }
}

impl Drop for RedbStoreInner {
    fn drop(&mut self) {
        utils::report_closing_flush(self.close(), &self.path);
    }
}

#[derive(Clone)]
pub struct RedbStore {
    inner: Arc<RedbStoreInner>,
}

impl std::fmt::Debug for RedbStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedbStore").finish_non_exhaustive()
    }
}

impl PartialEq for RedbStore {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}
impl Eq for RedbStore {}

impl RedbStore {
    pub fn open(
        config: StoreConfig,
        migration_set: MigrationSet,
    ) -> StorageResult<(Self, MigrationReport)> {
        let path: Arc<Path> = Arc::from(config.path.as_path());

        let opened = Arc::new(create_database(&config.path).doing(StorageError::Open, &path)?);

        let write_txn = opened.begin_write().doing(StorageError::Open, &path)?;
        {
            for table in [
                TABLE_DATA,
                TABLE_META,
                TABLE_DIFF_LOG,
                TABLE_MIGRATION_LOG,
                TABLE_SCHEMA_SNAPSHOT,
            ] {
                let _ = write_txn
                    .open_table(table)
                    .doing(StorageError::Open, &path)
                    .attach_table(table.name())?;
            }
        }
        write_txn.commit().doing(StorageError::Open, &path)?;

        let pending = Arc::new(Mutex::new(utils::Pending::new()));
        let initialized = Arc::new(Mutex::new(HashSet::<StorePath>::new()));
        let commits = Arc::new(CommitSignal::default());
        let subscriptions = Arc::new(RwLock::new(Vec::new()));

        let db = Arc::new(arc_swap::ArcSwapOption::from(Some(opened)));

        let db_save = db.clone();
        let pending_save = pending.clone();
        let path_save = path.clone();

        let write_lock = Arc::new(Mutex::new(()));
        let write_lock_save = write_lock.clone();

        let health = Arc::new(PersistHealth::default());

        let debouncer = Debouncer::new_with_retry(
            config.save_debounce,
            crate::store::debouncer::FlushPolicy {
                retry: config.retry_policy.clone(),
                commits: commits.clone(),
                health: health.clone(),
                on_giveup: config.on_persist_failure.clone(),
            },
            move || -> StorageResult<()> {
                let _write_guard = write_lock_save.lock();

                let changes = {
                    let lock = pending_save.lock();
                    if lock.is_empty() {
                        return Ok(());
                    }
                    lock.clone()
                };

                #[cfg(test)]
                if SIMULATE_WRITE_FAILURE.load(Ordering::Relaxed) {
                    return Err(error_stack::Report::new(StorageError::Flush)
                        .attach("simulated write failure"));
                }

                let landed: StorageResult<()> = (|| {
                    let db = db_save.load_full().ok_or_else(|| {
                        error_stack::Report::new(StorageError::Flush)
                            .attach("the database is being reopened")
                    })?;
                    let txn = db
                        .begin_write()
                        .doing(StorageError::Flush, &path_save)
                        .attach_buffered(changes.len())?;
                    apply_pending(&txn, &changes, &path_save)?;
                    txn.commit()
                        .doing(StorageError::Flush, &path_save)
                        .attach_buffered(changes.len())
                })();

                match landed {
                    Ok(()) => {
                        utils::clear_committed(&mut pending_save.lock(), &changes);
                        Ok(())
                    }
                    Err(report) => {
                        if is_previous_io(&report)
                            && let Err(failed) = reopen(&db_save, &path_save)
                        {
                            return Err(report
                                .attach("reopening the database after it stopped reaching the disk failed too")
                                .attach(failed));
                        }
                        Err(report)
                    }
                }
            },
        );

        let inner = Arc::new(RedbStoreInner {
            db,
            path,
            pending,
            initialized,
            commits,
            health,
            debouncer: Arc::new(debouncer),
            subscriptions,
            next_sub_id: Arc::new(AtomicU64::new(1)),
            write_lock,
            parallel_reads: config.parallel_reads,
            budget: Screening::resolve(&config.limits, Backend::Redb),
        });

        let store = Self { inner };
        let report = store
            .run_migrations(migration_set)
            .attach_store_file(&store.inner.path)
            .attach("opening the store")?;

        Ok((store, report))
    }

    pub fn close(&self) -> StorageResult<()> {
        self.inner.close()
    }

    /// The value a subscriber should see as the old one.
    ///
    /// The buffer wins where it has the key, since it holds the newer value;
    /// otherwise the committed one. Reading the buffer alone reported no old
    /// value once a flush had emptied it, though the key was on disk.
    fn committed_or_buffered(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        if let Some(op) = self.inner.pending.lock().get(path).filter(|o| o.is_data()) {
            return Ok(op.value().map(Vec::from));
        }

        let (_txn, table) = self.inner.read_data(StorageError::Read).attach_key(path)?;

        Ok(table
            .get(path.as_str())
            .doing(StorageError::Read, &self.inner.path)
            .attach_key(path)?
            .map(|v| Vec::from(&v.value()[..])))
    }
}

impl SchemaAwareStore for RedbStore {
    fn run_migrations(&self, mset: MigrationSet) -> StorageResult<MigrationReport> {
        struct RedbProvider<'a> {
            db: &'a Database,
            path: &'a Path,
        }

        impl<'a> StorageProvider for RedbProvider<'a> {
            fn atomic<F, T>(&self, f: F) -> StorageResult<T>
            where
                F: FnOnce(&mut dyn MigrationBackendAdapter) -> StorageResult<T>,
            {
                let write_txn = self
                    .db
                    .begin_write()
                    .doing(StorageError::Migrate, self.path)?;

                let res = {
                    let mut storage = RedbMigrationBackend::new(&write_txn, self.path);
                    f(&mut storage)?
                };

                write_txn.commit().doing(StorageError::Migrate, self.path)?;
                Ok(res)
            }
        }

        let db = self.inner.db()?;
        let provider = RedbProvider {
            db: &db,
            path: &self.inner.path,
        };
        let engine = MigrationEngine::new(&provider);
        engine.run(mset)
    }
}

impl StoreBackend for RedbStore {
    fn get_raw(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        {
            let lock = self.inner.pending.lock();
            if let Some(op) = lock.get(path).filter(|o| o.is_data()) {
                return Ok(op.value().map(|b| b.to_vec()));
            }
        }

        let (_txn, table) = self.inner.read_data(StorageError::Read).attach_key(path)?;
        match table
            .get(path.as_str())
            .doing(StorageError::Read, &self.inner.path)
            .attach_key(path)?
        {
            Some(access_guard) => Ok(Some(access_guard.value().to_vec())),
            None => Ok(None),
        }
    }

    fn get_erased(
        &self,
        path: &StorePath,
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<bool> {
        {
            let lock = self.inner.pending.lock();
            if let Some(op) = lock.get(path).filter(|o| o.is_data()) {
                return match op.value() {
                    Some(bytes) => {
                        self.decode_erased(bytes, f)
                            .change_context(StorageError::Read)
                            .attach_store_file(&self.inner.path)
                            .attach_key(path)
                            .attach("not yet flushed")?;
                        Ok(true)
                    }
                    None => Ok(false),
                };
            }
        }

        let (_txn, table) = self.inner.read_data(StorageError::Read).attach_key(path)?;
        match table
            .get(path.as_str())
            .doing(StorageError::Read, &self.inner.path)
            .attach_key(path)?
        {
            Some(access_guard) => {
                self.decode_erased(access_guard.value(), f)
                    .change_context(StorageError::Read)
                    .attach_store_file(&self.inner.path)
                    .attach_key(path)?;
                Ok(true)
            }
            None => Ok(false),
        }
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

    fn set_erased(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.set_owned_erased(path.clone(), value, source)
    }

    fn set_owned_erased(
        &self,
        path: StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.inner.check_debouncer()?;
        self.inner
            .budget
            .check_path(&path)
            .attach_store_file(&self.inner.path)?;

        let depth = self.inner.budget.for_value(&path);
        let counted = depth.count(value);
        let bytes = SERIALIZATION_BUFFER
            .with(|buf| {
                let mut b = buf.borrow_mut();
                b.clear();
                let mut ser = Serializer::new(&mut *b)
                    .with_bytes(BytesMode::ForceAll)
                    .with_struct_map();
                serde::Serialize::serialize(&counted, &mut ser).map_err(CodecError::from)?;

                Ok::<Vec<u8>, CodecError>(Vec::from(&b[..]))
            })
            .map_err(|e| {
                if depth.overflowed() {
                    self.inner
                        .budget
                        .too_deep(&path)
                        .attach(StoreFile(self.inner.path.to_path_buf()))
                } else {
                    error_stack::Report::new(e)
                        .change_context(StorageError::Codec)
                        .attach(StoreFile(self.inner.path.to_path_buf()))
                        .attach(Key(path.clone()))
                }
            })?;

        if let Some(refusal) = self.inner.budget.refused(&depth, &path) {
            return Err(refusal.attach(StoreFile(self.inner.path.to_path_buf())));
        }

        let old_bytes = self
            .committed_or_buffered(&path)
            .change_context(StorageError::Write)
            .attach_key(&path)
            .attach("reading the value a subscriber should see as the old one")?;

        if old_bytes.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }

        self.inner.buffer(|lock| {
            lock.insert(path.clone(), utils::PendingOp::Set(bytes.clone()));
        })?;

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path,
                op: StoreOp::Set,
                old: old_bytes,
                new: Some(bytes),
                source,
            },
        );

        self.inner.debouncer.schedule();
        Ok(())
    }

    fn parallel_reads(&self) -> bool {
        self.inner.parallel_reads
    }

    fn files(&self) -> Option<StoreLayout> {
        Some(StoreLayout::Single {
            data: self.inner.path.to_path_buf(),
        })
    }

    fn save_now(&self) -> StorageResult<()> {
        self.inner.save_now()
    }

    fn close(&self) -> StorageResult<()> {
        self.inner.close()
    }

    fn is_closed(&self) -> bool {
        self.inner.debouncer.is_stopped()
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        let subtree = prefix.subtree();

        let mut committed: Vec<(StorePath, Vec<u8>)> = Vec::new();

        let (_txn, table) = self.inner.read_data(StorageError::Scan).attach_prefix(prefix)?;

        let range = subtree.prefix()..;
        let entries = table
            .range(range)
            .doing(StorageError::Scan, &self.inner.path)
            .attach_prefix(prefix)?;
        for result in entries {
            let (k, v) = result
                .doing(StorageError::Scan, &self.inner.path)
                .attach_prefix(prefix)
                .attach_read_so_far(committed.len())?;
            let key_str = k.value();
            if subtree.contains(key_str) {
                committed.push((utils::stored_path(key_str)?, Vec::from(&v.value()[..])));
            } else if !key_str.starts_with(subtree.prefix()) {
                break;
            }
        }

        let mut buffered: Vec<(StorePath, Option<Vec<u8>>)> = {
            let lock = self.inner.pending.lock();
            lock.iter()
                .filter(|(key, op)| op.is_data() && subtree.contains(key.as_str()))
                .map(|(key, op)| (key.clone(), op.value().map(Vec::from)))
                .collect()
        };
        buffered.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

        Ok(utils::merge_buffered(committed, buffered))
    }

    /// The same answer `scan_prefix` gives, built nowhere.
    ///
    /// The engine's side is ranged over with a cursor and handed straight to
    /// the visitor - no path, no copy - while the buffer's side is collected
    /// and sorted first, because it is what is pending rather than what is
    /// stored and is small next to it. Merging the two is then one pass, and
    /// the order is the engine's, which is the order a scan promises.
    fn visit_prefix(
        &self,
        prefix: &StorePath,
        visit: &mut dyn FnMut(&str, &[u8]) -> StorageResult<()>,
    ) -> StorageResult<()> {
        let subtree = prefix.subtree();

        let mut buffered: Vec<(StorePath, Option<Vec<u8>>)> = {
            let lock = self.inner.pending.lock();
            lock.iter()
                .filter(|(key, op)| op.is_data() && subtree.contains(key.as_str()))
                .map(|(key, op)| (key.clone(), op.value().map(Vec::from)))
                .collect()
        };
        buffered.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

        let (_txn, table) = self.inner.read_data(StorageError::Scan).attach_prefix(prefix)?;
        let entries = table
            .range(subtree.prefix()..)
            .doing(StorageError::Scan, &self.inner.path)
            .attach_prefix(prefix)?;

        let mut pending = buffered.into_iter().peekable();

        for result in entries {
            let (k, v) = result
                .doing(StorageError::Scan, &self.inner.path)
                .attach_prefix(prefix)?;
            let key = k.value();

            if !subtree.contains(key) {
                if !key.starts_with(subtree.prefix()) {
                    break;
                }
                continue;
            }

            while pending.peek().is_some_and(|(p, _)| p.as_str() < key) {
                let (p, value) = pending.next().expect("peeked");
                if let Some(value) = value {
                    visit(p.as_str(), &value)?;
                }
            }

            match pending.peek() {
                Some((p, _)) if p.as_str() == key => {
                    let (p, value) = pending.next().expect("peeked");
                    if let Some(value) = value {
                        visit(p.as_str(), &value)?;
                    }
                }
                _ => visit(key, v.value())?,
            }
        }

        for (p, value) in pending {
            if let Some(value) = value {
                visit(p.as_str(), &value)?;
            }
        }

        Ok(())
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        let subtree = prefix.subtree();
        let mut keys: Vec<(StorePath, Vec<u8>)> = Vec::new();

        let read_txn = self
            .inner
            .db()?
            .begin_read()
            .doing(StorageError::Scan, &self.inner.path)
            .attach_prefix(prefix)?;

        let table = read_txn
            .open_table(TABLE_DATA)
            .doing(StorageError::Scan, &self.inner.path)
            .attach_table(TABLE_DATA.name())?;

        let entries = table
            .range(subtree.prefix()..)
            .doing(StorageError::Scan, &self.inner.path)
            .attach_prefix(prefix)?;

        for result in entries {
            let (k, _) = result
                .doing(StorageError::Scan, &self.inner.path)
                .attach_prefix(prefix)
                .attach_read_so_far(keys.len())?;
            let key = k.value();
            if !subtree.contains(key) {
                if !key.starts_with(subtree.prefix()) {
                    break;
                }
                continue;
            }
            keys.push((utils::stored_path(key)?, Vec::new()));
        }

        let mut buffered: Vec<(StorePath, Option<Vec<u8>>)> = {
            let lock = self.inner.pending.lock();
            lock.iter()
                .filter(|(key, op)| op.is_data() && subtree.contains(key.as_str()))
                .map(|(key, op)| (key.clone(), op.value().map(|_| Vec::new())))
                .collect()
        };
        buffered.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

        Ok(utils::merge_buffered(keys, buffered)
            .into_iter()
            .map(|(k, _)| k)
            .collect())
    }

    fn delete_with_source(&self, path: &StorePath, source: Option<Uuid>) -> StorageResult<()> {
        self.inner.check_debouncer()?;

        let old_bytes = self
            .committed_or_buffered(path)
            .change_context(StorageError::Delete)
            .attach_key(path)
            .attach("reading the value a subscriber should see as the old one")?;

        let Some(old_bytes) = old_bytes else {
            return Ok(());
        };

        self.inner.buffer(|lock| {
            lock.insert(path.clone(), utils::PendingOp::Delete);
        })?;

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path: path.clone(),
                op: StoreOp::Delete,
                old: Some(old_bytes),
                new: None,
                source,
            },
        );

        self.inner.debouncer.schedule();
        Ok(())
    }

    fn delete_prefix_with_source(
        &self,
        prefix: &StorePath,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.inner.check_debouncer()?;

        let keys = self
            .scan_prefix(prefix)
            .change_context(StorageError::Delete)
            .attach_prefix(prefix)
            .attach("listing the subtree to be removed")?;

        self.inner.buffer(|lock| {
            for (path, _) in keys {
                lock.insert(path, utils::PendingOp::Delete);
            }
        })?;

        utils::emit_events(
            &self.inner.subscriptions,
            StoreEvent {
                path: prefix.clone(),
                op: StoreOp::DeletePrefix,
                old: None,
                new: None,
                source,
            },
        );

        self.inner.debouncer.schedule();
        Ok(())
    }

    fn delete(&self, path: &StorePath) -> StorageResult<()> {
        self.delete_with_source(path, None)
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

    fn flush_prefix(&self, prefix: &StorePath) -> StorageResult<()> {
        self.inner.flush_prefix(prefix)
    }

    fn flush_async(&self) -> Commit {
        let commit = Commit::awaiting(self.inner.commits.clone());
        self.inner.debouncer.flush_now();
        commit
    }

    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool> {
        if self.inner.initialized.lock().contains(namespace.as_str()) {
            return Ok(true);
        }

        let key = utils::init_key(namespace.as_str());
        let read_txn = self
            .inner
            .db()?
            .begin_read()
            .doing(StorageError::Meta, &self.inner.path)
            .attach_prefix(namespace)?;
        let table = read_txn
            .open_table(TABLE_META)
            .doing(StorageError::Meta, &self.inner.path)
            .attach_table(TABLE_META.name())?;
        let found = table
            .get(key.as_str())
            .doing(StorageError::Meta, &self.inner.path)
            .attach_raw_key(&key)?
            .is_some();

        if found {
            self.inner.initialized.lock().insert(namespace.clone());
        }
        Ok(found)
    }

    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()> {
        if state.is_seeded() && self.inner.initialized.lock().contains(namespace.as_str()) {
            return Ok(());
        }

        self.inner.check_debouncer()?;
        self.inner.buffer(|lock| {
            lock.insert(namespace.clone(), utils::PendingOp::Init(state.is_seeded()));
        })?;

        let mut initialized = self.inner.initialized.lock();
        if state.is_seeded() {
            initialized.insert(namespace.clone());
        } else {
            initialized.remove(namespace);
        }
        drop(initialized);

        self.inner.debouncer.schedule();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::migration::fields::FieldDescriptor;
    use crate::migration::{MigrationError, MigrationPlan};
    use crate::store::IntoStorageReport;
    use crate::store::StoreExt;
    use crate::store::config::AfterGivingUp;
    use amethystate_core::test_utils::unique_path;
    use serial_test::serial;
    use std::thread;
    use std::time::Duration;

    const EMPTY_FIELDS: &[FieldDescriptor] = &[];

    #[test]
    #[serial]
    fn test_debouncer_persistence() {
        let path = unique_path("debounce");

        let mut config = StoreConfig::new(path);
        config.save_debounce = Duration::from_millis(50);

        let (store, _) = RedbStore::open(config, MigrationSet::default()).unwrap();

        store.set(["config", "port"], &8080u16).unwrap();

        {
            let read_txn = store.inner.db().unwrap().begin_read().unwrap();
            let table = read_txn.open_table(TABLE_DATA).unwrap();
            assert!(table.get("config.port").unwrap().is_none());
        }

        thread::sleep(Duration::from_millis(500));

        {
            let read_txn = store.inner.db().unwrap().begin_read().unwrap();
            let table = read_txn.open_table(TABLE_DATA).unwrap();
            assert!(table.get("config.port").unwrap().is_some());
        }
    }

    #[test]
    fn test_delete_flow() {
        let path = unique_path("delete");
        let (store, _) = RedbStore::open(StoreConfig::new(path), MigrationSet::default()).unwrap();

        store.set(["temp", "key"], &1).unwrap();

        store.save_now().unwrap();
        store
            .delete(&StorePath::from_segments(["temp", "key"]))
            .unwrap();
        assert_eq!(store.get::<i32>(["temp", "key"]).unwrap(), None);

        store.save_now().unwrap();

        let read_txn = store.inner.db().unwrap().begin_read().unwrap();
        let table = read_txn.open_table(TABLE_DATA).unwrap();
        assert!(table.get("temp.key").unwrap().is_none());
    }

    #[test]
    fn test_deterministic_closure_and_reopen() {
        let path = unique_path("closure");
        {
            let (store, _) =
                RedbStore::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();
            store.set(["test", "key"], &"hello".to_string()).unwrap();
            store.close().expect("Explicit close failed");
        }

        let (store_reopened, _) = RedbStore::open(StoreConfig::new(&path), MigrationSet::default())
            .expect("Database should be available immediately after close");

        let val: Option<String> = store_reopened.get(["test", "key"]).unwrap();
        assert_eq!(val, Some("hello".to_string()));
    }

    #[test]
    fn test_drop_behavior_is_deterministic() {
        let path = unique_path("drop_logic");
        {
            let (store, _) =
                RedbStore::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();
            store.set(["drop", "test"], &42u32).unwrap();
        }

        let (store_reopened, _) = RedbStore::open(StoreConfig::new(&path), MigrationSet::default())
            .expect("Drop must release file lock deterministically");

        assert_eq!(
            store_reopened.get::<u32>(["drop", "test"]).unwrap(),
            Some(42)
        );
    }

    #[test]
    fn test_close_saves_pending_data() {
        let path = unique_path("save_on_close");
        let mut config = StoreConfig::new(&path);
        config.save_debounce = Duration::from_secs(3600);

        {
            let (store, _) = RedbStore::open(config, MigrationSet::default()).unwrap();
            store.set(["urgent", "data"], &true).unwrap();
            store.close().unwrap();
        }

        let (store, _) = RedbStore::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();
        assert_eq!(store.get::<bool>(["urgent", "data"]).unwrap(), Some(true));
    }

    #[test]
    fn test_granular_flush_prefix_drains_buffer() {
        let path = unique_path("granular_flush");
        let mut config = StoreConfig::new(&path);

        config.save_debounce = Duration::from_secs(3600);

        let (store, _) = RedbStore::open(config, MigrationSet::default()).unwrap();

        store
            .set(["net", "host"], &"127.0.0.1".to_string())
            .unwrap();
        store.set(["net", "port"], &8080u16).unwrap();
        store.set(["ui", "theme"], &"dark".to_string()).unwrap();

        {
            let pending = store.inner.pending.lock();
            assert_eq!(pending.len(), 3);
        }
        {
            let read_txn = store.inner.db().unwrap().begin_read().unwrap();
            let table = read_txn.open_table(TABLE_DATA).unwrap();
            assert!(table.get("net.host").unwrap().is_none());
            assert!(table.get("ui.theme").unwrap().is_none());
        }

        store
            .flush_prefix(&StorePath::from_segments(["net"]))
            .unwrap();

        {
            let read_txn = store.inner.db().unwrap().begin_read().unwrap();
            let table = read_txn.open_table(TABLE_DATA).unwrap();
            assert_eq!(
                store
                    .decode::<String>(table.get("net.host").unwrap().unwrap().value())
                    .unwrap(),
                "127.0.0.1"
            );
            assert_eq!(
                store
                    .decode::<u16>(table.get("net.port").unwrap().unwrap().value())
                    .unwrap(),
                8080
            );
            assert!(
                table.get("ui.theme").unwrap().is_none(),
                "UI should remain in the RAM buffer"
            );
        }

        {
            let pending = store.inner.pending.lock();
            assert_eq!(
                pending.len(),
                1,
                "Only ui.theme should remain in the buffer"
            );
            assert!(pending.contains_key("ui.theme"));
            assert!(!pending.contains_key("net.host"));
            assert!(!pending.contains_key("net.port"));
        }

        store.flush_prefix(&StorePath::root()).unwrap();
        {
            let pending = store.inner.pending.lock();
            assert!(
                pending.is_empty(),
                "Pending buffer should be completely empty"
            );
        }
        {
            let read_txn = store.inner.db().unwrap().begin_read().unwrap();
            let table = read_txn.open_table(TABLE_DATA).unwrap();
            assert!(
                table.get("ui.theme").unwrap().is_some(),
                "UI should now be persisted on disk"
            );
        }
    }

    #[test]
    fn test_component_atomic_rollback() {
        let path = unique_path("rollback");
        let mut cfg = StoreConfig::new(&path);
        cfg.save_debounce = Duration::from_millis(50);
        {
            let (store, _) = RedbStore::open(cfg, MigrationSet::default()).unwrap();
            store.set(["net", "ip"], &"1.1.1.1".to_string()).unwrap();
            store.save_now().unwrap();
        }

        let mset = MigrationSet::default()
            .add(
                "net",
                MigrationPlan::new().step(1, "ok", |ctx| ctx.set("ip", &"8.8.8.8".to_string())),
                0,
                EMPTY_FIELDS,
                &[],
            )
            .add(
                "ui",
                MigrationPlan::new().step(1, "fail", |_| {
                    Err(MigrationError::Custom("crash".into()).into_report())
                }),
                0,
                EMPTY_FIELDS,
                &["net"],
            );

        let (store, report) = RedbStore::open(StoreConfig::new(&path), mset).unwrap();
        assert!(report.has_failures());

        let val: String = store.get(["net", "ip"]).unwrap().unwrap();
        assert_eq!(val, "1.1.1.1");
    }
    #[test]
    #[serial]
    fn test_debouncer_retains_buffer_on_simulated_transaction_failure() {
        let path = unique_path("debouncer_simulated_fail");

        let mut config = StoreConfig::new(&path);
        config.save_debounce = Duration::from_millis(50);

        SIMULATE_WRITE_FAILURE.store(true, Ordering::Relaxed);

        let (store, _) = RedbStore::open(config, MigrationSet::default()).unwrap();

        let test_key = StorePath::from_segments(["system", "critical_update"]);
        let test_value = "payload_data".to_string();
        store.set(&test_key, &test_value).unwrap();

        {
            let pending = store.inner.pending.lock();
            assert!(pending.contains_key(test_key.as_str()));
        }

        thread::sleep(Duration::from_millis(150));

        SIMULATE_WRITE_FAILURE.store(false, Ordering::Relaxed);

        {
            let pending = store.inner.pending.lock();
            assert!(
                pending.contains_key(test_key.as_str()),
                "The pending changes buffer should not be cleared when a transaction fails!"
            );
        }

        let retrieved: Option<String> = store.get(&test_key).unwrap();
        assert_eq!(retrieved, Some(test_value));
    }

    /// Waits for `condition`, and says whether it came true.
    ///
    /// These tests used to sleep two hundred milliseconds against a fifty
    /// millisecond budget and assert afterwards. That is a guess rather than a
    /// bound on anything: it passes while the machine is quiet and fails when
    /// the binary is busy, which is exactly when a test is least useful. The
    /// deadline here is long enough that reaching it means the thing genuinely
    /// did not happen, and a run on a quiet machine returns as soon as it does.
    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        condition()
    }

    fn failing_store(tag: &str, decision: AfterGivingUp) -> (RedbStore, Arc<Mutex<Vec<String>>>) {
        let mut config = StoreConfig::new(unique_path(tag));
        config.save_debounce = Duration::from_millis(10);
        config.retry_policy = crate::store::config::RetryPolicy {
            interval: Duration::from_millis(10),
            budget: Duration::from_millis(50),
        };

        let heard: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let heard_write = heard.clone();
        config.on_persist_failure = Some(Arc::new(
            move |reason: &error_stack::Report<StorageError>| {
                heard_write.lock().push(format!("{reason:#}"));
                decision
            },
        ));

        let (store, _) = RedbStore::open(config, MigrationSet::default()).unwrap();
        (store, heard)
    }

    /// A flush that keeps failing tells writers so, once its streak has
    /// outlived the budget - an error they can act on, not a dead process.
    /// A full disk is somebody about to delete something, and taking the
    /// application down with it is the store's least useful reaction.
    #[test]
    #[serial]
    fn a_flush_that_keeps_failing_fails_the_next_write_rather_than_the_process() {
        SIMULATE_WRITE_FAILURE.store(true, Ordering::Relaxed);
        let (store, heard) = failing_store("debouncer_fails_writes", AfterGivingUp::Fail);

        store
            .set(StorePath::from_segments(["doomed"]), &1u32)
            .unwrap();

        assert!(
            wait_until(|| !heard.lock().is_empty()),
            "on_persist_failure never ran once the streak outlived the budget"
        );
        assert!(
            !store.inner.debouncer.is_poisoned(),
            "a disk that will not take a write is not a reason to poison the writer"
        );

        let refused = store.set(StorePath::from_segments(["another"]), &2u32);
        assert!(
            refused.is_err(),
            "a write while the flush is not landing should say so, not queue quietly"
        );

        assert_eq!(
            store
                .get::<u32>(StorePath::from_segments(["doomed"]))
                .unwrap(),
            Some(1),
            "the reads the store already had are untouched by any of it"
        );

        SIMULATE_WRITE_FAILURE.store(false, Ordering::Relaxed);
    }

    /// And it heals: the disk comes back, the next flush lands, and writes
    /// work again with nothing restarted.
    #[test]
    #[serial]
    fn a_disk_that_comes_back_heals_the_store() {
        SIMULATE_WRITE_FAILURE.store(true, Ordering::Relaxed);
        let (store, _) = failing_store("debouncer_heals", AfterGivingUp::Fail);

        store
            .set(StorePath::from_segments(["waiting"]), &1u32)
            .unwrap();
        assert!(
            wait_until(|| store.inner.health.failure().is_some()),
            "the flush never gave up, so there is nothing for a write to be \
             refused by"
        );
        assert!(
            store
                .set(StorePath::from_segments(["nope"]), &2u32)
                .is_err(),
            "the store should be refusing writes before the disk comes back"
        );

        SIMULATE_WRITE_FAILURE.store(false, Ordering::Relaxed);

        assert!(
            wait_until(|| store.inner.health.failure().is_none()),
            "a flush that can land again never cleared the failure"
        );
        store
            .set(StorePath::from_segments(["fine"]), &3u32)
            .expect("writes should work again once a flush has landed");
    }

    /// The application that would rather stop than run on with state it
    /// cannot persist can still say so.
    #[test]
    #[serial]
    fn poison_is_available_for_an_application_that_asks_for_it() {
        SIMULATE_WRITE_FAILURE.store(true, Ordering::Relaxed);
        let (store, _) = failing_store("debouncer_poisons", AfterGivingUp::Poison);

        store
            .set(StorePath::from_segments(["doomed"]), &1u32)
            .unwrap();

        assert!(
            wait_until(|| store.inner.debouncer.is_poisoned()),
            "AfterGivingUp::Poison should have taken the writer down"
        );

        let poisoned_write = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.set(StorePath::from_segments(["another"]), &2u32)
        }));
        assert!(
            poisoned_write.is_err(),
            "a write after a poison should panic in the caller's own stack"
        );

        SIMULATE_WRITE_FAILURE.store(false, Ordering::Relaxed);
    }

    /// The flush a short-lived process depends on is the one nobody is left to
    /// ask about: `Drop` has no caller to hand an error to. It leaves a line
    /// instead, and this is what fails if that line ever goes away.
    #[test]
    #[serial]
    #[tracing_test::traced_test]
    fn a_closing_flush_that_fails_leaves_a_trace() {
        let path = unique_path("redb_closing_flush");
        let _disk = recovery::arm_failing_disk(&path);

        let mut config = StoreConfig::new(&path);
        config.save_debounce = Duration::from_secs(60);
        let (store, _) = RedbStore::open(config, MigrationSet::default()).unwrap();

        store
            .set(StorePath::from_segments(["lost"]), &1u32)
            .unwrap();

        recovery::WRITES_LEFT.store(0, Ordering::SeqCst);
        drop(store);

        assert!(
            logs_contain("the store's closing flush failed"),
            "a store that could not write on the way out said nothing"
        );
    }
}
