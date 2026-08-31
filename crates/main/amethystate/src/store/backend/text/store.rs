use super::document::TextDocument;
use super::error::TextStoreError;
use crate::MigrationReport;
use crate::errors::StorageError;
use crate::migration::engine::{MigrationEngine, StorageProvider};
use crate::migration::set::MigrationSet;
use crate::store::backend::text::migration::TextMigrationBackend;
use crate::store::backend::utils;
use crate::store::backend::utils::Attempted;
use crate::store::config::{FileWritePolicy, StoreConfig};
use crate::store::screening::{Noticed, Screening};
use crate::store::durable::{Commit, CommitSignal, PersistHealth};
use crate::store::facts::{Facts, Key, StoreFile as StoreFileFact};
use crate::store::traits::{MigrationBackendAdapter, StoreLayout};
use crate::store::debouncer::Debouncer;
use crate::store::{
    EXTERNAL_EDIT, InitState, SchemaAwareStore, StorageResult, StoreBackend, StoreCallback,
    StoreEvent, StoreOp, SubscriptionEntry, SubscriptionId, SubscriptionKind,
};
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::{Mutex, RwLock};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::NamedTempFile;
use tracing::{info, warn};

trait InMetaFile: ResultExt {
    fn in_meta(self, what: StorageError, file: &Path) -> StorageResult<Self::Ok>;
}

impl<R: ResultExt> InMetaFile for R {
    fn in_meta(self, what: StorageError, file: &Path) -> StorageResult<Self::Ok> {
        self.change_context(what).attach_meta_file(file)
    }
}

pub struct StoreFile<D> {
    pub path: PathBuf,
    pub backup_path: PathBuf,
    pub doc: Arc<RwLock<D>>,
    pub write_policy: FileWritePolicy,
    /// Held across rendering the document *and* replacing the file, so two
    /// flushes cannot interleave.
    ///
    /// Each replacement is atomic on its own, which buys nothing once there are
    /// two writers: the debouncer's thread and a `save_now` from anywhere would
    /// both render, then both replace, and whichever replaced second won -
    /// leaving the file holding what the *first* one saw. `save_now` returning
    /// `Ok` meant this thread's replacement landed, not that it is still there.
    flush: Arc<Mutex<()>>,
}

impl<D> Clone for StoreFile<D> {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            backup_path: self.backup_path.clone(),
            doc: self.doc.clone(),
            write_policy: self.write_policy,
            flush: self.flush.clone(),
        }
    }
}

/// One record's key in the metadata file, which is flat.
///
/// Reading the data file needs the schema, and the schema is in here - so this
/// file cannot be laid out by a rule that has to be read out of it. Joining
/// once and storing the result whole keeps it readable with no schema at all.
pub(super) fn meta_key(kind: &str, path: &StorePath) -> StorePath {
    StorePath::segment(kind).join(path)
}

impl<D: TextDocument> StoreFile<D> {
    pub fn new(path: PathBuf, initial_doc: D, write_policy: FileWritePolicy) -> Self {
        let backup_path = match path.file_name() {
            Some(name) => {
                let mut name = name.to_os_string();
                name.push(".bak");
                path.with_file_name(name)
            }
            None => path.with_extension("bak"),
        };
        Self {
            path,
            backup_path,
            doc: Arc::new(RwLock::new(initial_doc)),
            write_policy,
            flush: Arc::new(Mutex::new(())),
        }
    }

    pub fn create_backup(&self) -> StorageResult<()> {
        if self.path.exists() {
            std::fs::copy(&self.path, &self.backup_path)
                .map_err(TextStoreError::from)
                .change_context(StorageError::Open)
                .attach_store_file(&self.path)
                .attach_with(|| format!("backup: {}", self.backup_path.display()))?;
        }
        Ok(())
    }

    /// Reads the file, and backs up only what it could read.
    ///
    /// The backup is taken after the read rather than before it, because the
    /// copy exists to hold a readable file: a previous open that died partway
    /// through a migration leaves a good backup beside a half-written data
    /// file, and copying that file over the backup destroys the only intact
    /// copy - in exactly the case the backup is kept for.
    ///
    /// So a file that will not parse leaves the backup alone and is recovered
    /// from it when it holds something readable.
    pub fn load_and_back_up(&self) -> StorageResult<D> {
        match self.load_or_empty() {
            Ok(doc) => {
                self.create_backup()?;
                Ok(doc)
            }
            Err(unreadable) => match self.recover_from_backup() {
                Some(doc) => {
                    warn!(
                        path = %self.path.display(),
                        backup = %self.backup_path.display(),
                        "the file could not be read and was restored from the backup a \
                         previous open left behind"
                    );
                    Ok(doc)
                }
                None => Err(unreadable),
            },
        }
    }

    fn recover_from_backup(&self) -> Option<D> {
        if !self.backup_path.exists() {
            return None;
        }

        let content = std::fs::read_to_string(&self.backup_path).ok()?;
        let doc = D::parse(&content).ok()?;
        std::fs::copy(&self.backup_path, &self.path).ok()?;

        Some(doc)
    }

    pub fn load_or_empty(&self) -> StorageResult<D> {
        if self.path.exists() {
            let content = std::fs::read_to_string(&self.path)
                .map_err(TextStoreError::from)
                .change_context(StorageError::Open)
                .attach_store_file(&self.path)?;
            D::parse(&content).attach_store_file(&self.path)
        } else {
            Ok(D::empty())
        }
    }

    /// Renders the document and replaces the file with it, as one step.
    ///
    /// The lock covers both halves rather than the read alone. A guard taken
    /// only for the render is released before the replacement, which is where
    /// two flushes used to cross: A renders, B renders, B replaces, A replaces,
    /// and the file ends up holding what A saw.
    pub fn persist(&self) -> StorageResult<()> {
        let _flushing = self.flush.lock();

        let content = self
            .doc
            .read()
            .serialize()
            .attach_store_file(&self.path)?;
        persist_atomic(&self.path, &content, self.write_policy)
            .map_err(TextStoreError::from)
            .change_context(StorageError::Flush)
            .attach_store_file(&self.path)?;
        Ok(())
    }

    pub fn restore_from_backup(&self, fallback_to_initial: &D) {
        *self.doc.write() = fallback_to_initial.clone();

        if self.backup_path.exists() {
            let _ = std::fs::copy(&self.backup_path, &self.path);
            let _ = std::fs::remove_file(&self.backup_path);
        } else if self.path.exists() {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub fn clean_backup(&self) {
        if self.backup_path.exists() {
            let _ = std::fs::remove_file(&self.backup_path);
        }
    }
}

pub struct StoreFiles<D: TextDocument> {
    pub data: StoreFile<D>,
    pub meta: StoreFile<D>,
}

impl<D: TextDocument> Clone for StoreFiles<D> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            meta: self.meta.clone(),
        }
    }
}

impl<D: TextDocument> StoreFiles<D> {
    pub fn load_and_back_up(&self) -> StorageResult<(D, D)> {
        let data = self
            .data
            .load_and_back_up()
            .attach("role: the store's data")?;
        let meta = self
            .meta
            .load_and_back_up()
            .attach("role: the store's schema bookkeeping")?;
        Ok((data, meta))
    }

    pub fn persist(&self) -> StorageResult<()> {
        self.data.persist().attach("role: the store's data")?;
        self.meta
            .persist()
            .attach("role: the store's schema bookkeeping")?;
        Ok(())
    }

    pub fn clean_backups(&self) {
        self.data.clean_backup();
        self.meta.clean_backup();
    }

    pub fn restore_from_backups(&self, fallback_data: &D, fallback_meta: &D) {
        self.data.restore_from_backup(fallback_data);
        self.meta.restore_from_backup(fallback_meta);
    }
}

pub(crate) struct TextStoreInner<D: TextDocument> {
    pub(crate) files: StoreFiles<D>,
    pub(crate) subscriptions: Arc<RwLock<Vec<SubscriptionEntry>>>,
    pub(crate) next_id: Arc<AtomicU64>,
    pub(crate) debouncer: Arc<Debouncer>,
    pub(crate) commits: Arc<CommitSignal>,
    pub(crate) health: Arc<PersistHealth>,
    /// Bumped by every mutation, and compared against `persisted` to tell
    /// whether the document differs from the file. A flag could not do this:
    /// checking it and acting on it are two steps, and a write landing in
    /// between was either lost or clobbered.
    pub(crate) writes: Arc<AtomicU64>,
    pub(crate) persisted: Arc<AtomicU64>,
    /// What this store may spend on a path and its value together, worked out
    /// once from the codec's own ceiling and whatever the caller promised.
    pub(crate) budget: Screening,
    watch_debouncer: Arc<Debouncer>,
    _watcher: RecommendedWatcher,
}

impl<D: TextDocument> TextStoreInner<D> {
    /// Whether a write may proceed.
    ///
    /// A background flush that has been failing past its budget is an error
    /// the caller can act on, not a reason to take the process down - the
    /// value is refused, what is already buffered keeps being retried, and a
    /// flush that lands clears this. A debouncer thread that is actually dead
    /// is a different thing and still panics: that is a bug here, not a disk.
    pub(crate) fn check_debouncer(&self) -> StorageResult<()> {
        utils::check_debouncer(&self.health, &self.debouncer)
    }
}

impl<D: TextDocument> Drop for TextStoreInner<D> {
    fn drop(&mut self) {
        utils::report_closing_flush(self.close(), &self.files.data.path);
    }
}

#[derive(Clone)]
pub struct TextStore<D: TextDocument> {
    pub(crate) inner: Arc<TextStoreInner<D>>,
}

impl<D: TextDocument> PartialEq for TextStore<D> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}
impl<D: TextDocument> Eq for TextStore<D> {}

impl<D: TextDocument> Debug for TextStore<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextStore")
            .field("data_path", &self.inner.files.data.path)
            .field("meta_path", &self.inner.files.meta.path)
            .finish()
    }
}

impl<D: TextDocument + Send + 'static> TextStore<D> {
    pub fn open(
        config: StoreConfig,
        migration_set: MigrationSet,
    ) -> StorageResult<(Self, MigrationReport)> {
        let path = config.path.clone();
        let meta_path = config.path.with_extension("meta");

        let files = StoreFiles {
            data: StoreFile::new(path, D::empty(), config.file_write),
            meta: StoreFile::new(meta_path, D::empty(), config.file_write),
        };

        let (initial_data, initial_meta) = files.load_and_back_up()?;

        *files.data.doc.write() = initial_data.clone();
        *files.meta.doc.write() = initial_meta.clone();

        let store = Self::new(config, files)?;

        match store.run_migrations(migration_set) {
            Ok(report) => {
                store.inner.files.persist()?;
                store.inner.files.clean_backups();
                Ok((store, report))
            }
            Err(e) => {
                store
                    .inner
                    .files
                    .restore_from_backups(&initial_data, &initial_meta);
                Err(e
                    .attach(StoreFileFact(store.inner.files.data.path.clone()))
                    .attach("the files were restored from their backups"))
            }
        }
    }

    fn new(config: StoreConfig, files: StoreFiles<D>) -> StorageResult<Self> {
        info!(
            path = %config.path.display(),
            "initializing TextStore"
        );

        let subscriptions = Arc::new(RwLock::new(Vec::<SubscriptionEntry>::new()));
        let writes = Arc::new(AtomicU64::new(0));
        let persisted = Arc::new(AtomicU64::new(0));

        let files_debounce = files.clone();
        let writes_debounce = writes.clone();
        let persisted_debounce = persisted.clone();
        let commits = Arc::new(CommitSignal::default());

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
                let saving = writes_debounce.load(Ordering::Acquire);
                files_debounce.persist()?;
                persisted_debounce.store(saving, Ordering::Release);
                Ok(())
            },
        );

        let files_watch = files.clone();
        let watch_subs = subscriptions.clone();
        let writes_watch = writes.clone();
        let persisted_watch = persisted.clone();
        let meta_path = files.meta.path.clone();

        let watch_debouncer = Arc::new(Debouncer::new(config.watch_debounce, move || {
            sync_external_changes::<D>(
                &files_watch.data,
                &watch_subs,
                &writes_watch,
                &persisted_watch,
            );

            if let Ok(content) = std::fs::read_to_string(&meta_path)
                && let Ok(on_disk) = D::parse(&content)
            {
                let guard = files_watch.meta.doc.read();
                let current_str = guard.serialize().unwrap_or_default();
                let on_disk_str = on_disk.serialize().unwrap_or_default();
                if current_str != on_disk_str {
                    warn!(
                        "⚠️  External modification of metadata file detected! \
                         Metadata must only be mutated via internal migrations."
                    );
                }
            }
        }));

        let watch_debouncer_trigger = watch_debouncer.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            let Ok(event) = res else { return };

            let is_modify = matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_));
            if !is_modify {
                return;
            }

            watch_debouncer_trigger.schedule();
        })
        .map_err(|e| TextStoreError::Watch(e.to_string()))
        .change_context(StorageError::Open)
        .attach_store_file(&config.path)?;

        let watch_dir = config.path.parent().unwrap_or(Path::new("."));
        let mut watcher = watcher;
        watcher
            .watch(watch_dir, RecursiveMode::NonRecursive)
            .map_err(|e| TextStoreError::Watch(e.to_string()))
            .change_context(StorageError::Open)
            .attach_with(|| format!("watching: {}", watch_dir.display()))
            .attach_store_file(&config.path)?;

        let inner = Arc::new(TextStoreInner {
            files,
            subscriptions,
            next_id: Arc::new(AtomicU64::new(1)),
            debouncer: Arc::new(debouncer),
            commits,
            health,
            writes,
            persisted,
            budget: Screening::for_codec(&config.limits, D::format()),
            watch_debouncer,
            _watcher: watcher,
        });

        Ok(Self { inner })
    }
}

impl<D: TextDocument + Send + 'static> SchemaAwareStore for TextStore<D> {
    fn run_migrations(&self, mset: MigrationSet) -> StorageResult<MigrationReport> {
        struct TextProvider<D: TextDocument> {
            data_doc: Arc<RwLock<D>>,
            meta_doc: Arc<RwLock<D>>,
        }

        impl<D: TextDocument> StorageProvider for TextProvider<D> {
            fn atomic<F, T>(&self, f: F) -> StorageResult<T>
            where
                F: FnOnce(&mut dyn MigrationBackendAdapter) -> StorageResult<T>,
            {
                let mut data_guard = self.data_doc.write();
                let mut meta_guard = self.meta_doc.write();

                let backup_data = data_guard.clone();
                let backup_meta = meta_guard.clone();

                let mut storage = TextMigrationBackend {
                    data_doc: &mut *data_guard,
                    meta_doc: &mut *meta_guard,
                };

                match f(&mut storage) {
                    Ok(val) => Ok(val),
                    Err(e) => {
                        *data_guard = backup_data;
                        *meta_guard = backup_meta;
                        Err(e)
                    }
                }
            }
        }

        let provider = TextProvider {
            data_doc: self.inner.files.data.doc.clone(),
            meta_doc: self.inner.files.meta.doc.clone(),
        };
        let engine = MigrationEngine::new(&provider);
        engine
            .run(mset)
            .doing(StorageError::Migrate, &self.inner.files.data.path)
            .attach_meta_file(&self.inner.files.meta.path)
    }
}

impl<D: TextDocument> TextStoreInner<D> {
    fn get_node_bytes(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        self.refuse_if_closed()?;
        let guard = self.files.data.doc.read();
        let levels: Vec<Cow<'_, str>> = path.segments().collect();
        let parts: Vec<&str> = levels.iter().map(Cow::as_ref).collect();
        match guard.get(&parts) {
            Some(node) => Ok(Some(
                D::node_to_bytes(node)
                    .doing(StorageError::Read, &self.files.data.path)
                    .attach_key(path)?,
            )),
            None => Ok(None),
        }
    }

    fn set_erased_inner(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.check_debouncer()?;
        self.budget
            .check_path(path)
            .attach_store_file(&self.files.data.path)?;

        let depth = self.budget.for_value(path);
        let node = D::serialize_node(value, &depth).map_err(|e| {
            if depth.overflowed() {
                self.budget
                    .too_deep(path)
                    .attach(StoreFileFact(self.files.data.path.clone()))
            } else {
                e.change_context(StorageError::Write)
                    .attach(Key(path.clone()))
            }
        })?;

        if let Some(refusal) = self.budget.refused(&depth, path) {
            return Err(refusal.attach(StoreFileFact(self.files.data.path.clone())));
        }

        self.set_node(path.clone(), node, source)
    }

    fn save_now(&self) -> StorageResult<()> {
        let saving = self.writes.load(Ordering::Acquire);
        self.files.persist()?;
        self.persisted.store(saving, Ordering::Release);
        Ok(())
    }

    /// Renders the document one last time and stops both background threads.
    ///
    /// There is no handle to give up here - a document engine writes through a
    /// temporary file and holds nothing open between flushes - so what closing
    /// settles is the threads. The watcher's own debouncer goes too: a file
    /// changing underneath a closed store has nobody left to tell.
    ///
    /// Closing twice is fine: the second call finds the thread stopped and
    /// returns, so `Drop` after an explicit close does nothing.
    pub(crate) fn close(&self) -> StorageResult<()> {
        {
            let _data = self.files.data.doc.write();
            let _meta = self.files.meta.doc.write();
            if !self.debouncer.stop_accepting() {
                return Ok(());
            }
        }

        self.debouncer.shutdown();
        self.watch_debouncer.shutdown();
        self.save_now().attach("rendering the document before close")
    }

    /// Refuses a read or a write once the store has closed.
    ///
    /// A document engine keeps the whole store in memory, so a closed one
    /// could go on answering reads from it. It does not: an engine holding a
    /// file answers `Closed` there, and a store that reads on one engine and
    /// refuses on another is worse than either.
    ///
    /// A write calls it with the document already locked, because closing
    /// takes that same lock to decide it is closing: a write is either in the
    /// document before that decision - and so in the render that follows it -
    /// or it is refused. Checked before the lock, a write lands after the last
    /// render and is reported as taken while never reaching the file.
    fn refuse_if_closed(&self) -> StorageResult<()> {
        if self.debouncer.is_stopped() {
            return Err(error_stack::Report::new(StorageError::Closed)
                .attach(StoreFileFact(self.files.data.path.clone())));
        }
        Ok(())
    }

    /// Picks up an edit made to the file outside the process before writing our
    /// own, unless we have unsaved changes of our own to lose.
    pub(crate) fn pull_external_changes(&self) {
        sync_external_changes::<D>(
            &self.files.data,
            &self.subscriptions,
            &self.writes,
            &self.persisted,
        );
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        self.refuse_if_closed()?;
        let guard = self.files.data.doc.read();
        scan_prefix_impl(&*guard, prefix)
            .attach_store_file(&self.files.data.path)
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        self.refuse_if_closed()?;
        let guard = self.files.data.doc.read();
        scan_keys_impl(&*guard, prefix)
            .attach_store_file(&self.files.data.path)
    }

    fn delete(&self, path: &StorePath, source: Option<uuid::Uuid>) -> StorageResult<()> {
        self.check_debouncer()?;

        self.pull_external_changes();

        let levels: Vec<Cow<'_, str>> = path.segments().collect();
        let parts: Vec<&str> = levels.iter().map(Cow::as_ref).collect();

        let old_bytes = {
            let mut guard = self.files.data.doc.write();
            self.refuse_if_closed()?;
            let old = guard
                .get(&parts)
                .map(|n| D::node_to_bytes(n))
                .transpose()
                .doing(StorageError::Delete, &self.files.data.path)
                .attach_key(path)?;
            guard
                .delete(&parts)
                .doing(StorageError::Delete, &self.files.data.path)
                .attach_key(path)?;
            if old.is_some() {
                self.writes.fetch_add(1, Ordering::Release);
            }
            old
        };

        let Some(old_bytes) = old_bytes else {
            return Ok(());
        };

        utils::emit_events(
            &self.subscriptions,
            StoreEvent {
                path: path.clone(),
                op: StoreOp::Delete,
                old: Some(old_bytes),
                new: None,
                source,
            },
        );

        self.debouncer.schedule();
        Ok(())
    }

    fn delete_prefix(&self, prefix: &StorePath, source: Option<uuid::Uuid>) -> StorageResult<()> {
        self.check_debouncer()?;

        self.pull_external_changes();

        {
            let levels: Vec<Cow<'_, str>> = prefix.segments().collect();
            let parts: Vec<&str> = levels.iter().map(Cow::as_ref).collect();
            let mut guard = self.files.data.doc.write();
            guard
                .delete_subtree(&parts)
                .doing(StorageError::Delete, &self.files.data.path)
                .attach_prefix(prefix)?;
            self.writes.fetch_add(1, Ordering::Release);
        }

        utils::emit_events(
            &self.subscriptions,
            StoreEvent {
                path: prefix.clone(),
                op: StoreOp::DeletePrefix,
                old: None,
                new: None,
                source,
            },
        );

        self.debouncer.schedule();
        Ok(())
    }

    fn subscribe(&self, kind: SubscriptionKind, callback: StoreCallback) -> SubscriptionId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.subscriptions
            .write()
            .push(SubscriptionEntry { id, kind, callback });
        id
    }

    fn unsubscribe(&self, id: SubscriptionId) {
        self.subscriptions.write().retain(|s| s.id != id);
    }

    fn init_key(&self, namespace: &StorePath) -> StorePath {
        meta_key("__init", namespace)
    }

    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool> {
        self.refuse_if_closed()?;
        let key = self.init_key(namespace);
        let guard = self.files.meta.doc.read();
        Ok(guard.get(&[key.as_str()]).is_some())
    }

    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()> {
        let key = self.init_key(namespace);
        {
            let mut guard = self.files.meta.doc.write();
            self.refuse_if_closed()?;
            let parts = [key.as_str()];

            match state {
                InitState::Seeded => {
                    let node = D::serialize_node(&true, &Noticed::unlimited())
                        .in_meta(StorageError::Meta, &self.files.meta.path)
                        .attach_key(namespace)?;
                    guard.set(&parts, node)
                }
                InitState::Fresh => guard.delete(&parts).map(|_| ()),
            }
            .in_meta(StorageError::Meta, &self.files.meta.path)
            .attach_key(namespace)?;
        }

        self.files
            .meta
            .persist()
            .change_context(StorageError::Meta)
            .attach_key(namespace)?;
        Ok(())
    }

    /// Writes `node` at `path_str`, reporting a removal if the document does
    /// not keep it - a format with no way to write nothing answers a `None`
    /// with an absent key.
    pub(crate) fn set_node(
        &self,
        path_str: StorePath,
        node: D::Node,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.pull_external_changes();

        let levels: Vec<Cow<'_, str>> = path_str.segments().collect();
        let parts: Vec<&str> = levels.iter().map(Cow::as_ref).collect();
        let (old_bytes, new_bytes) = {
            let mut guard = self.files.data.doc.write();
            self.refuse_if_closed()?;
            let old = guard
                .get(&parts)
                .map(|n| D::node_to_bytes(n))
                .transpose()
                .doing(StorageError::Write, &self.files.data.path)
                .attach_key(&path_str)
                .attach("while reading the value being replaced")?;

            let incoming = D::node_to_bytes(&node)
                .doing(StorageError::Write, &self.files.data.path)
                .attach_key(&path_str)
                .attach("while comparing the write against what is already stored")?;

            if old.as_deref() == Some(incoming.as_slice()) {
                return Ok(());
            }

            guard
                .set(&parts, node)
                .doing(StorageError::Write, &self.files.data.path)
                .attach_key(&path_str)?;
            let new = guard
                .get(&parts)
                .map(|n| D::node_to_bytes(n))
                .transpose()
                .doing(StorageError::Write, &self.files.data.path)
                .attach_key(&path_str)?;

            self.writes.fetch_add(1, Ordering::Release);
            (old, new)
        };

        let event = match new_bytes {
            Some(new) => StoreEvent {
                path: path_str.clone(),
                op: StoreOp::Set,
                old: old_bytes,
                new: Some(new),
                source,
            },
            None => {
                let Some(old) = old_bytes else {
                    self.debouncer.schedule();
                    return Ok(());
                };
                StoreEvent {
                    path: path_str.clone(),
                    op: StoreOp::Delete,
                    old: Some(old),
                    new: None,
                    source,
                }
            }
        };

        utils::emit_events(&self.subscriptions, event);

        self.debouncer.schedule();
        Ok(())
    }
}

impl<D: TextDocument + Send + 'static> StoreBackend for TextStore<D> {
    fn get_raw(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        self.inner.get_node_bytes(path)
    }

    fn get_erased(
        &self,
        path: &StorePath,
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<bool> {
        match self.inner.get_node_bytes(path)? {
            Some(bytes) => {
                D::with_bytes_de(&bytes, f)
                    .doing(StorageError::Read, &self.inner.files.data.path)
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
        D::with_bytes_de(bytes, f)
            .attach_store_file(&self.inner.files.data.path)
    }

    fn set_erased(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.inner.set_erased_inner(path, value, source)
    }

    fn set_owned_erased(
        &self,
        path: StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.inner.set_erased_inner(&path, value, source)
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

    fn files(&self) -> Option<StoreLayout> {
        let data = &self.inner.files.data;
        let meta = &self.inner.files.meta;

        Some(StoreLayout::Sidecars {
            data: data.path.clone(),
            meta: meta.path.clone(),
            data_backup: data.backup_path.clone(),
            meta_backup: meta.backup_path.clone(),
        })
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        self.inner.scan_prefix(prefix)
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        self.inner.scan_keys(prefix)
    }

    fn delete(&self, path: &StorePath) -> StorageResult<()> {
        self.delete_with_source(path, None)
    }

    fn delete_with_source(
        &self,
        path: &StorePath,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.inner.delete(path, source)
    }

    fn delete_prefix_with_source(
        &self,
        prefix: &StorePath,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.inner.delete_prefix(prefix, source)
    }

    fn subscribe(&self, kind: SubscriptionKind, callback: StoreCallback) -> SubscriptionId {
        self.inner.subscribe(kind, callback)
    }

    fn unsubscribe(&self, id: SubscriptionId) {
        self.inner.unsubscribe(id)
    }

    fn flush_async(&self) -> Commit {
        let commit = Commit::awaiting(self.inner.commits.clone());
        self.inner.debouncer.flush_now();
        commit
    }

    /// Saves the whole document, whatever prefix was asked for.
    ///
    /// The store is one file and rendering any of it renders all of it, so
    /// there is no narrower thing to do. Holding some keys back would mean
    /// building a second document to write and re-reading it afterwards, which
    /// buys a caller nothing and exists only to have the document engines
    /// behave like the database ones.
    ///
    /// [`Backend::a_commit_covers_the_whole_store`] is where that is written
    /// down, and it is what the durability tests ask rather than each naming
    /// the answer for its own engine.
    ///
    /// [`Backend::a_commit_covers_the_whole_store`]: crate::store::builder::Backend::a_commit_covers_the_whole_store
    fn flush_prefix(&self, prefix: &StorePath) -> StorageResult<()> {
        self.save_now().attach_prefix(prefix)
    }

    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool> {
        self.inner.is_initialized(namespace)
    }

    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()> {
        self.inner.set_initialized(namespace, state)
    }
}

/// Writes `content` where `path` names, so that a reader sees either the whole
/// of it or none.
///
/// The temporary file is made in the target's own directory, because a
/// replacement has to sit on the same volume, and the contents are flushed
/// before the name is moved: otherwise the rename can reach the disk while the
/// bytes are still in the write-back cache, which is how a config file comes
/// back truncated after a power cut. Windows offers no write-through on the
/// replacement itself, so the flush has to be ours.
///
/// A replacement that has to be retried takes the same temporary file back from
/// the failure and tries again with it: the contents are written and flushed
/// already, and only the name is in dispute.
///
/// How long each of the two steps is worth is [`FileWritePolicy`], because what
/// is holding the file is the application's business and not this function's.
fn persist_atomic(path: &Path, content: &str, policy: FileWritePolicy) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let dir = path.parent().unwrap_or(Path::new("."));

    let mut written = None;
    for attempt in 0..policy.write.attempts.max(1) {
        match write_temp(dir, content) {
            Ok(tmp) => {
                written = Some(tmp);
                break;
            }
            Err(e) if attempt + 1 >= policy.write.attempts => return Err(e),
            Err(_) => std::thread::sleep(policy.write.pause),
        }
    }
    let mut tmp = written.expect("the loop above returns rather than falling through");

    for attempt in 0..policy.replace.attempts.max(1) {
        match tmp.persist(path) {
            Ok(_) => return Ok(()),
            Err(e) if attempt + 1 >= policy.replace.attempts => return Err(e.error),
            Err(e) => {
                tmp = e.file;
                std::thread::sleep(policy.replace.pause);
            }
        }
    }
    unreachable!("the loop above returns on its last attempt")
}

/// The contents in a file of their own, beside the target and already on the
/// disk.
fn write_temp(dir: &Path, content: &str) -> io::Result<NamedTempFile> {
    let mut tmp = NamedTempFile::new_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.as_file().sync_all()?;
    Ok(tmp)
}

pub(super) fn scan_prefix_impl<D: TextDocument>(
    doc: &D,
    prefix: &StorePath,
) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
    let levels: Vec<Cow<'_, str>> = prefix.segments().collect();
    let parts: Vec<&str> = levels.iter().map(Cow::as_ref).collect();
    let target_depth = parts.len() + 1;
    let mut raw_nodes = Vec::new();
    scan_prefix_recursive(
        doc,
        &parts,
        prefix.as_str(),
        &mut raw_nodes,
        Some(target_depth),
    )?;

    let mut results = Vec::new();
    for (k, node) in raw_nodes {
        if k.starts_with(prefix.as_str()) {
            let bytes = D::node_to_bytes(&node)
                .change_context(StorageError::Scan)
                .attach_prefix(prefix)
                .attach_raw_key(&k)?;
            results.push((utils::stored_path(&k)?, bytes));
        }
    }

    results.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
    Ok(results)
}

pub(super) fn scan_prefix_recursive<D: TextDocument>(
    doc: &D,
    parts: &[&str],
    prefix_str: &str,
    results: &mut Vec<(String, D::Node)>,
    target_depth: Option<usize>,
) -> StorageResult<()> {
    let current_depth = parts.len();

    if let Some(target_depth) = target_depth
        && current_depth >= target_depth
    {
        if !prefix_str.is_empty()
            && let Some(node) = doc.get(parts)
        {
            results.push((prefix_str.to_string(), node.clone()));
        }
        return Ok(());
    }

    let children = doc.scan(parts)?;
    if children.is_empty() {
        if !prefix_str.is_empty()
            && let Some(node) = doc.get(parts)
        {
            results.push((prefix_str.to_string(), node.clone()));
        }
    } else {
        for (full_key, _node) in children {
            let child_path = utils::stored_path(&full_key)?;
            let child_levels: Vec<Cow<'_, str>> = child_path.segments().collect();
            let child_parts: Vec<&str> = child_levels.iter().map(Cow::as_ref).collect();
            let grand_children = doc.scan(&child_parts)?;

            let should_stop = grand_children.is_empty()
                || target_depth.is_some_and(|depth| child_parts.len() >= depth);

            if should_stop {
                if let Some(child_node) = doc.get(&child_parts) {
                    results.push((full_key, child_node.clone()));
                }
            } else {
                scan_prefix_recursive(doc, &child_parts, prefix_str, results, target_depth)?;
            }
        }
    }

    Ok(())
}

/// Takes an edit made to the file outside the process, unless doing so would
/// throw away a write of our own.
///
/// The file is read before the document is locked, so what came back can be a
/// version behind by the time the decision is made: a flush landing in that gap
/// leaves the reader holding a document older than the one on disk, and
/// installing it drops every write the flush had just saved. Which write
/// counter the flush had reached is therefore read before the file and checked
/// again under the lock, and a value that moved means the content is stale.
///
/// The other half of the guarantee is that a write raises `writes` while it
/// still holds the document lock. Raised after, there is a window where the
/// change is in the document and the store still looks saved, and a reader
/// arriving inside it is entitled to overwrite it.
fn sync_external_changes<D: TextDocument>(
    file: &StoreFile<D>,
    subscriptions: &Arc<RwLock<Vec<SubscriptionEntry>>>,
    writes: &AtomicU64,
    persisted: &AtomicU64,
) {
    let read_after = persisted.load(Ordering::Acquire);

    let Ok(content) = std::fs::read_to_string(&file.path) else {
        return;
    };
    let Ok(on_disk) = D::parse(&content) else {
        return;
    };

    let events = {
        let mut guard = file.doc.write();

        let written = writes.load(Ordering::Acquire);
        let saved = persisted.load(Ordering::Acquire);

        if written != saved || saved != read_after {
            return;
        }

        let old_serialized = guard.serialize().unwrap_or_default();
        let new_serialized = on_disk.serialize().unwrap_or_default();
        if old_serialized == new_serialized {
            Vec::new()
        } else {
            let old = guard.clone();
            *guard = on_disk;
            info!("external store change detected");
            match diff_documents::<D>(&old, &*guard) {
                Ok(events) => events,
                Err(e) => {
                    tracing::error!(
                        "an external edit could not be read, so nobody was told about it: {e:?}"
                    );
                    return;
                }
            }
        }
    };
    for event in events {
        utils::emit_events(subscriptions, event);
    }
}

fn diff_documents<D: TextDocument>(old: &D, new: &D) -> StorageResult<Vec<StoreEvent>> {
    let mut old_nodes = Vec::new();
    scan_prefix_recursive(old, &[], "", &mut old_nodes, None)
        .attach("reading the document as it was before the edit")?;
    let old_map: HashMap<String, D::Node> = old_nodes.into_iter().collect();

    let mut new_nodes = Vec::new();
    scan_prefix_recursive(new, &[], "", &mut new_nodes, None)
        .attach("reading the document as it is on disk")?;
    let new_map: HashMap<String, D::Node> = new_nodes.into_iter().collect();

    let mut events = Vec::new();

    let mut all_keys: std::collections::BTreeSet<String> = old_map.keys().cloned().collect();
    all_keys.extend(new_map.keys().cloned());

    for key in all_keys {
        let old_node = old_map.get(&key);
        let new_node = new_map.get(&key);

        match (old_node, new_node) {
            (Some(o), Some(n)) => {
                let old_bytes = D::node_to_bytes(o).ok();
                let new_bytes = D::node_to_bytes(n).ok();
                if old_bytes != new_bytes {
                    events.push(StoreEvent {
                        path: utils::stored_path(&key)?,
                        op: StoreOp::Set,
                        old: old_bytes,
                        new: new_bytes,
                        source: Some(EXTERNAL_EDIT),
                    });
                }
            }
            (Some(o), None) => {
                let old_bytes = D::node_to_bytes(o).ok();
                events.push(StoreEvent {
                    path: utils::stored_path(&key)?,
                    op: StoreOp::Delete,
                    old: old_bytes,
                    new: None,
                    source: Some(EXTERNAL_EDIT),
                });
            }
            (None, Some(n)) => {
                let new_bytes = D::node_to_bytes(n).ok();
                events.push(StoreEvent {
                    path: utils::stored_path(&key)?,
                    op: StoreOp::Set,
                    old: None,
                    new: new_bytes,
                    source: Some(EXTERNAL_EDIT),
                });
            }
            (None, None) => {}
        }
    }

    Ok(events)
}

pub(super) fn scan_keys_impl<D: TextDocument>(
    doc: &D,
    prefix: &StorePath,
) -> StorageResult<Vec<StorePath>> {
    let levels: Vec<Cow<'_, str>> = prefix.segments().collect();
    let parts: Vec<&str> = levels.iter().map(Cow::as_ref).collect();
    let target_depth = parts.len() + 1;
    let mut keys = Vec::new();
    scan_keys_recursive(doc, &parts, prefix.as_str(), &mut keys, Some(target_depth))?;

    keys.retain(|k| k.starts_with(prefix.as_str()));
    keys.sort();

    keys.iter().map(|k| utils::stored_path(k)).collect()
}

fn scan_keys_recursive<D: TextDocument>(
    doc: &D,
    parts: &[&str],
    prefix_str: &str,
    keys: &mut Vec<String>,
    target_depth: Option<usize>,
) -> StorageResult<()> {
    let current_depth = parts.len();

    if let Some(target_depth) = target_depth
        && current_depth >= target_depth
    {
        if !prefix_str.is_empty() && doc.get(parts).is_some() {
            keys.push(prefix_str.to_string());
        }
        return Ok(());
    }

    let children = doc.scan(parts)?;
    if children.is_empty() {
        if !prefix_str.is_empty() && doc.get(parts).is_some() {
            keys.push(prefix_str.to_string());
        }
    } else {
        for (full_key, _node) in children {
            let child_path = utils::stored_path(&full_key)?;
            let child_levels: Vec<Cow<'_, str>> = child_path.segments().collect();
            let child_parts: Vec<&str> = child_levels.iter().map(Cow::as_ref).collect();
            let grand_children = doc.scan(&child_parts)?;

            let should_stop = grand_children.is_empty()
                || target_depth.is_some_and(|depth| child_parts.len() >= depth);

            if should_stop {
                if doc.get(&child_parts).is_some() {
                    keys.push(full_key);
                }
            } else {
                scan_keys_recursive(doc, &child_parts, prefix_str, keys, target_depth)?;
            }
        }
    }

    Ok(())
}
