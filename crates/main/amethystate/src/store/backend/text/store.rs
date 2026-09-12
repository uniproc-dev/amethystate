use super::document::{Navigable, TextDocument};
use super::error::TextStoreError;
use super::files::{StoreFile, StoreFiles, has_no_keys};
use super::standoff::{Standoff, save};
use crate::MigrationReport;
use crate::errors::StorageError;
use crate::migration::engine::{MigrationEngine, StorageProvider};
use crate::migration::set::MigrationSet;
use crate::store::backend::text::layout;
use crate::store::backend::text::migration::TextMigrationBackend;
use crate::store::backend::text::watching;
use crate::store::backend::utils;
use crate::store::backend::utils::Attempted;
use crate::store::backend::utils::refuse_closing_from_a_flush;
use crate::store::config::StoreConfig;
use crate::store::debouncer::{Debouncer, FlushPolicy};
use crate::store::declared::{Declared, Holds};
use crate::store::durable::{Commit, CommitSignal, PersistHealth};
use crate::store::facts::{Facts, Key, StoreFile as StoreFileFact};
use crate::store::format::{self, StorageFactSet};
use crate::store::meta::SchemaSnapshot;
use crate::store::screening::{Noticed, Screening};
use crate::store::traits::{MigrationBackendAdapter, StoreLayout};
use crate::store::{
    InitState, SchemaAwareStore, StorageResult, StoreBackend, StoreCallback, StoreEvent, StoreOp,
    SubscriptionEntry, SubscriptionId, SubscriptionKind, WhenItWillNotRead,
};
use amethystate_core::Source;
use amethystate_core::path::{SmolStr, StorePath, Stored};
use error_stack::ResultExt;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::RwLock;
use std::fmt::Debug;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, warn};

trait InMetaFile: ResultExt {
    fn in_meta(self, what: StorageError, file: &Path) -> StorageResult<Self::Ok>;
}

impl<R: ResultExt> InMetaFile for R {
    fn in_meta(self, what: StorageError, file: &Path) -> StorageResult<Self::Ok> {
        self.change_context(what).attach_meta_file(file)
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

/// Where a record named `key` sits in the metadata file, which is one level
/// deep: the whole key is the name.
///
/// The one place the joining is undone into a name again, and the reason
/// [`meta_key`] can go on being a path - a report names the record it is about,
/// and the file holds it whole.
pub(super) fn meta_at(key: &StorePath) -> StorePath {
    key.as_one_level()
}

/// What a record named `key` is about, or `None` for a record of another kind.
///
/// The inverse of [`meta_key`] against the same flat file. A record is one
/// level whose name is the joined key, so a scan hands that name back escaped
/// and it has to be read as a path again before the kind can be taken off it.
pub(super) fn meta_subject(kind: &str, key: &StorePath) -> StorageResult<Option<StorePath>> {
    let Some(name) = key.name() else {
        return Ok(None);
    };

    let named = StorePath::parse_joined(name.as_str())
        .change_context(StorageError::Path)
        .attach_key(key)?;

    Ok(named.strip_prefix(&StorePath::segment(kind)))
}

/// Puts back a declared map's level where deleting its last entry took it.
///
/// A document drops a level nothing is left in, which is right for a level that
/// existed only to hold what was deleted. A declared map is not that: its level
/// is the map, and whether it stands is how a store whose bookkeeping went
/// missing tells a map somebody emptied from one that was never written at all.
/// See [`Declared::owns_level`].
/// The level it put back, so the caller can record having written it.
///
/// It is a write like any other: a save that has to lay this store's own
/// changes over what the file holds replays what was recorded and nothing
/// else, so a level put back here and not recorded is a level the next
/// lay-over prunes away again - the delete that emptied it is replayed, the
/// putting back is not, and the map is gone.
fn keep_a_declared_level<D: TextDocument>(
    doc: &mut D,
    declared: &Declared,
    at: &StorePath,
) -> StorageResult<Option<StorePath>> {
    let Some(level) = at.parent() else {
        return Ok(None);
    };

    if !declared.owns_level(&level) || doc.get(&level).is_some() {
        return Ok(None);
    }

    doc.set(
        &level,
        <D::Node as super::document::Navigable>::make_empty_map(),
    )?;

    Ok(Some(level))
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

    pub(crate) standoff: Arc<Standoff>,

    /// What a save does when the file it is about to replace will not read.
    pub(crate) will_not_read: WhenItWillNotRead,

    /// The order the document was changed in, minted under the lock that
    /// settles it. Every change carries it, an edit read back off the file
    /// included, so a value built out of these ends where the store did.
    pub(crate) settled: Arc<AtomicU64>,

    /// What the closing flush did, for the closes that follow it.
    pub(crate) closed: utils::Closed,
    /// What this store may spend on a path and its value together, worked out
    /// once from the codec's own ceiling and whatever the caller promised.
    pub(crate) budget: Screening,
    /// Where the declarations put the edge of a value, so a scan knows what to
    /// take whole.
    ///
    /// Built on the first scan and dropped when a migration records new
    /// schemas, because half of it is read out of the file those go into.
    pub(crate) declared: RwLock<Option<Arc<Declared>>>,

    /// Whether this open found data with no bookkeeping beside it.
    ///
    /// Judged once, on the files as they were read, because everything after
    /// that puts keys back into the metadata: the format record is settled
    /// before a migration runs, and the schemas are recorded as the structs
    /// are built. See [`MigrationBackendAdapter::bookkeeping_is_lost`].
    pub(crate) bookkeeping_is_lost: bool,

    /// The quiet period the watcher waits out before it looks, held here so
    /// dropping this store ends a wait already in progress. See
    /// [`watching::Coalescing`].
    settling: Arc<watching::Coalescing>,
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
        self.settling.stop();
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

        let (initial_data, initial_meta) = match files.load() {
            Ok(read) => read,
            Err(why)
                if super::super::utils::start_fresh(
                    &config,
                    crate::store::builder::Backend::writing(D::format()),
                    &why,
                ) =>
            {
                files.load()?
            }
            Err(why) => return Err(why),
        };

        let bookkeeping_is_lost = has_no_keys(&initial_meta) && !has_no_keys(&initial_data);

        *files.data.doc.write() = initial_data.clone();
        *files.meta.doc.write() = initial_meta.clone();

        let store = Self::new(config, files, bookkeeping_is_lost)?;
        format::settle_for_codec(&store, D::format())
            .attach_store_file(&store.inner.files.data.path)
            .attach("opening the store")?;

        store.inner.files.take_backups()?;

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

    fn new(
        config: StoreConfig,
        files: StoreFiles<D>,
        bookkeeping_is_lost: bool,
    ) -> StorageResult<Self> {
        info!(
            path = %config.path.display(),
            "initializing TextStore"
        );

        let subscriptions = Arc::new(RwLock::new(Vec::<SubscriptionEntry>::new()));
        let writes = Arc::new(AtomicU64::new(0));
        let persisted = Arc::new(AtomicU64::new(0));

        let standoff = Arc::new(Standoff::default());
        let will_not_read = config.will_not_read;
        let settled = Arc::new(AtomicU64::new(0));
        let settled_watch = settled.clone();
        let settled_debounce = settled.clone();

        let files_debounce = files.clone();
        let subs_debounce = subscriptions.clone();
        let standoff_debounce = standoff.clone();
        let writes_debounce = writes.clone();
        let persisted_debounce = persisted.clone();
        let commits = Arc::new(CommitSignal::default());

        let health = Arc::new(PersistHealth::default());

        let debouncer = Debouncer::new_with_retry(
            config.save_debounce,
            FlushPolicy {
                retry: config.retry_policy.clone(),
                commits: commits.clone(),
                health: health.clone(),
                on_giveup: config.on_persist_failure.clone(),
                unsaved: {
                    let held = standoff.clone();
                    Arc::new(move || held.unsaved())
                },
            },
            move || -> StorageResult<()> {
                save(
                    &files_debounce,
                    &subs_debounce,
                    &writes_debounce,
                    &persisted_debounce,
                    &standoff_debounce,
                    &settled_debounce,
                    will_not_read,
                )
            },
        );

        let files_watch = files.clone();
        let watch_subs = subscriptions.clone();
        let writes_watch = writes.clone();
        let persisted_watch = persisted.clone();
        let standoff_watch = standoff.clone();
        let meta_path = files.meta.path.clone();

        let settling = watching::Coalescing::new(config.watch_debounce);
        let settling_watch = settling.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            let Ok(event) = res else { return };

            let is_modify = matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_));
            if !is_modify {
                return;
            }

            if !settling_watch.settle() {
                return;
            }

            watching::take_outside_edit::<D>(
                &files_watch.data,
                &watch_subs,
                &writes_watch,
                &persisted_watch,
                &standoff_watch,
                &settled_watch,
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
            standoff,
            will_not_read,
            settled,
            closed: utils::Closed::default(),
            budget: Screening::for_codec(&config.limits, D::format()),
            declared: RwLock::new(None),
            bookkeeping_is_lost,
            settling,
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
            bookkeeping_is_lost: bool,
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
                    bookkeeping_is_lost: self.bookkeeping_is_lost,
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
            bookkeeping_is_lost: self.inner.bookkeeping_is_lost,
        };
        let engine = MigrationEngine::new(&provider);
        let ran = engine
            .run(mset)
            .doing(StorageError::Migrate, &self.inner.files.data.path)
            .attach_meta_file(&self.inner.files.meta.path);

        self.inner.forget_declared();
        ran
    }
}

impl<D: TextDocument> TextStoreInner<D> {
    fn get_node_bytes(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        self.refuse_if_closed()?;
        let declared = self.declared()?;
        let guard = self.files.data.doc.read();
        let at = layout::levels(&*guard, &declared, path);
        match guard.get(&at) {
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
        save(
            &self.files,
            &self.subscriptions,
            &self.writes,
            &self.persisted,
            &self.standoff,
            &self.settled,
            self.will_not_read,
        )
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
        refuse_closing_from_a_flush()?;
        {
            let _data = self.files.data.doc.write();
            let _meta = self.files.meta.doc.write();
            if !self.debouncer.stop_accepting() {
                return self.closed.again(&self.files.data.path);
            }
        }

        self.debouncer.shutdown();
        self.closed.settled(
            self.save_now()
                .attach("rendering the document before close"),
        )
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
        watching::take_outside_edit::<D>(
            &self.files.data,
            &self.subscriptions,
            &self.writes,
            &self.persisted,
            &self.standoff,
            &self.settled,
        );
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        self.refuse_if_closed()?;
        let declared = self.declared()?;
        let guard = self.files.data.doc.read();
        scan_prefix_impl(&*guard, prefix, &declared).attach_store_file(&self.files.data.path)
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        self.refuse_if_closed()?;
        let declared = self.declared()?;
        let guard = self.files.data.doc.read();
        scan_paths_impl(&*guard, prefix, &declared).attach_store_file(&self.files.data.path)
    }

    /// The declarations a scan reads, this binary's and the store's own.
    pub(crate) fn declared(&self) -> StorageResult<Arc<Declared>> {
        if let Some(known) = self.declared.read().clone() {
            return Ok(known);
        }

        let read =
            declared_in(&*self.files.meta.doc.read()).attach_meta_file(&self.files.meta.path)?;

        let built = Arc::new(read);
        *self.declared.write() = Some(built.clone());
        Ok(built)
    }

    /// Forgets what the declarations said, for a migration that has just
    /// written new ones.
    pub(crate) fn forget_declared(&self) {
        *self.declared.write() = None;
    }

    /// The schemas the store recorded, whichever binary wrote them.
    pub(crate) fn recorded_schemas(&self) -> StorageResult<Vec<(StorePath, SchemaSnapshot)>> {
        recorded_schemas(&*self.files.meta.doc.read()).attach_meta_file(&self.files.meta.path)
    }

    fn delete(&self, path: &StorePath, source: Option<uuid::Uuid>) -> StorageResult<()> {
        self.check_debouncer()?;

        self.pull_external_changes();

        let declared = self.declared()?;

        let (old_bytes, settled) = {
            let mut guard = self.files.data.doc.write();
            self.refuse_if_closed()?;
            let at = layout::levels(&*guard, &declared, path);
            let old = guard
                .get(&at)
                .map(|n| D::node_to_bytes(n))
                .transpose()
                .doing(StorageError::Delete, &self.files.data.path)
                .attach_key(path)?;
            guard
                .delete(&at)
                .doing(StorageError::Delete, &self.files.data.path)
                .attach_key(path)?;
            let kept = keep_a_declared_level(&mut *guard, &declared, &at)
                .doing(StorageError::Delete, &self.files.data.path)
                .attach_key(path)?;
            if old.is_some() {
                self.standoff.wrote(&at, path);
                if let Some(level) = kept {
                    self.standoff.wrote(&level, path);
                }
                self.writes.fetch_add(1, Ordering::Release);
            }
            (old, self.settled.fetch_add(1, Ordering::AcqRel) + 1)
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
                source: source.into(),
                at: settled,
            },
        )?;

        self.debouncer.schedule();
        Ok(())
    }

    fn delete_prefix(&self, prefix: &StorePath, source: Option<uuid::Uuid>) -> StorageResult<()> {
        self.check_debouncer()?;

        self.pull_external_changes();

        let declared = self.declared()?;
        let settled = {
            let mut guard = self.files.data.doc.write();
            self.refuse_if_closed()?;

            let (plane, _) = at_the_root(&*guard, &declared, prefix)?;

            for at in plane {
                let key = at.as_one_level();
                guard
                    .delete(&key)
                    .doing(StorageError::Delete, &self.files.data.path)
                    .attach_key(&at)?;
                self.standoff.wrote(&key, &at);
            }

            guard
                .delete_subtree(prefix)
                .doing(StorageError::Delete, &self.files.data.path)
                .attach_prefix(prefix)?;
            self.standoff.swept(prefix);
            self.writes.fetch_add(1, Ordering::Release);
            self.settled.fetch_add(1, Ordering::AcqRel) + 1
        };

        utils::emit_events(
            &self.subscriptions,
            StoreEvent {
                path: prefix.clone(),
                op: StoreOp::DeletePrefix,
                old: None,
                new: None,
                source: source.into(),
                at: settled,
            },
        )?;

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
        meta_key("init", namespace)
    }

    pub(crate) fn read_format_facts(&self) -> StorageResult<Option<StorageFactSet>> {
        let record = format::RECORD;
        let guard = self.files.meta.doc.read();

        let Some(node) = guard.get(&StorePath::segment(record)) else {
            return Ok(None);
        };

        D::deserialize_node(node)
            .in_meta(StorageError::Meta, &self.files.meta.path)
            .attach_meta_node(record)
            .map(Some)
    }

    pub(crate) fn write_format_facts(&self, facts: &StorageFactSet) -> StorageResult<()> {
        let record = format::RECORD;
        let node = D::serialize_node(facts, &Noticed::unlimited())
            .in_meta(StorageError::Meta, &self.files.meta.path)
            .attach_meta_node(record)?;

        {
            let mut guard = self.files.meta.doc.write();
            guard
                .set(&StorePath::segment(record), node)
                .in_meta(StorageError::Meta, &self.files.meta.path)
                .attach_meta_node(record)?;
        }

        self.debouncer.schedule();
        Ok(())
    }

    /// Whether this namespace has been seeded, asking the bookkeeping first
    /// and the data second.
    ///
    /// The marker lives in the metadata file, which is a second file and can
    /// go missing on its own. Where the namespace is a declared level, the
    /// data answers as well: a map that was written and then emptied leaves
    /// the level standing with nothing in it, and a map that never existed
    /// leaves no level at all. That is the one bit, recovered from the file
    /// that holds the data rather than from the file that was lost.
    ///
    /// A namespace nothing declares is a plane of whole keys with no level of
    /// its own, so emptying it leaves nothing to read and the marker is all
    /// there is.
    ///
    /// The marker says `false` where a reset has been asked for, and that is
    /// an answer rather than the absence of one: the store was told to forget,
    /// and a level the reset left standing does not overrule it.
    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool> {
        self.refuse_if_closed()?;
        let key = self.init_key(namespace);

        let marked = self
            .files
            .meta
            .doc
            .read()
            .get(&meta_at(&key))
            .map(D::deserialize_node::<bool>);

        if let Some(Ok(seeded)) = marked {
            return Ok(seeded);
        }

        let declared = self.declared()?;
        if !declared.covers(namespace) {
            return Ok(false);
        }

        Ok(self.files.data.doc.read().get(namespace).is_some())
    }

    fn record_schema(&self, at: &StorePath, schema: &SchemaSnapshot) -> StorageResult<()> {
        let key = meta_key("schema", at);

        {
            let mut guard = self.files.meta.doc.write();
            self.refuse_if_closed()?;
            let parts = meta_at(&key);

            let mut held: Vec<SchemaSnapshot> = match guard.get(&parts) {
                Some(node) => D::deserialize_node(node)
                    .in_meta(StorageError::Meta, &self.files.meta.path)
                    .attach_meta_node(&key)?,
                None => Vec::new(),
            };

            if !crate::store::moved::record_into(&mut held, schema) {
                return Ok(());
            }

            let node = D::serialize_node(&held, &Noticed::unlimited())
                .in_meta(StorageError::Meta, &self.files.meta.path)
                .attach_meta_node(&key)?;

            guard
                .set(&parts, node)
                .in_meta(StorageError::Meta, &self.files.meta.path)
                .attach_meta_node(&key)?;
        }

        self.forget_declared();
        self.files
            .meta
            .persist()
            .change_context(StorageError::Meta)
            .attach_key(at)
    }

    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()> {
        let key = self.init_key(namespace);
        {
            let mut guard = self.files.meta.doc.write();
            self.refuse_if_closed()?;
            let parts = meta_at(&key);

            // Written either way, `false` included. A reset is something the
            // store was told, and the data can be read as "seeded" on its own
            // - see `is_initialized` - so leaving nothing here would let a
            // level the reset stepped over answer for it.
            let node = D::serialize_node(&state.is_seeded(), &Noticed::unlimited())
                .in_meta(StorageError::Meta, &self.files.meta.path)
                .attach_key(namespace)?;

            guard
                .set(&parts, node)
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

        let declared = self.declared()?;
        let (old_bytes, new_bytes, settled) = {
            let mut guard = self.files.data.doc.write();
            self.refuse_if_closed()?;
            let at = layout::levels(&*guard, &declared, &path_str);
            let old = guard
                .get(&at)
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
                .set(&at, node)
                .doing(StorageError::Write, &self.files.data.path)
                .attach_key(&path_str)?;
            let new = guard
                .get(&at)
                .map(|n| D::node_to_bytes(n))
                .transpose()
                .doing(StorageError::Write, &self.files.data.path)
                .attach_key(&path_str)?;

            self.standoff.wrote(&at, &path_str);
            self.writes.fetch_add(1, Ordering::Release);
            (old, new, self.settled.fetch_add(1, Ordering::AcqRel) + 1)
        };

        let event = match new_bytes {
            Some(new) => StoreEvent {
                path: path_str.clone(),
                op: StoreOp::Set,
                old: old_bytes,
                new: Some(new),
                source: source.into(),
                at: settled,
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
                    source: source.into(),
                    at: settled,
                }
            }
        };

        utils::emit_events(&self.subscriptions, event)?;

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
        D::with_bytes_de(bytes, f).attach_store_file(&self.inner.files.data.path)
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

    #[cfg(feature = "test-utils")]
    fn format_record(&self) -> Option<&dyn crate::store::format::TestFormatRecord> {
        Some(self)
    }

    #[cfg(feature = "test-utils")]
    fn reread_from_disk(&self) {
        self.inner.pull_external_changes();
    }

    fn files_layout(&self) -> Option<StoreLayout> {
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

    fn record_schema(&self, at: &StorePath, schema: &SchemaSnapshot) -> StorageResult<()> {
        self.inner.record_schema(at, schema)
    }
}

/// What this binary declares, and what the store recorded on top of it.
///
/// A store opened by a tool that declares nothing of its own has only the
/// second half, which is the whole reason the schemas are written down.
pub(super) fn declared_in<D: TextDocument>(meta: &D) -> StorageResult<Declared> {
    let mut declared = Declared::compiled_in().clone();

    for (prefix, snapshot) in recorded_schemas(meta)? {
        declared.record(&prefix, &snapshot.fields);
    }

    Ok(declared)
}

/// The schemas written into the bookkeeping file, by prefix.
pub(super) fn recorded_schemas<D: TextDocument>(
    meta: &D,
) -> StorageResult<Vec<(StorePath, SchemaSnapshot)>> {
    let mut found = Vec::new();

    for (key, node) in meta.scan(&StorePath::root())? {
        let Some(prefix) = meta_subject("schema", &key)? else {
            continue;
        };

        let trees: Vec<SchemaSnapshot> = D::deserialize_node(&node)
            .change_context(StorageError::Meta)
            .attach_key(&key)?;

        found.extend(trees.into_iter().map(|tree| (prefix.clone(), tree)));
    }

    Ok(found)
}

/// Every path stored under `prefix`, sorted.
///
/// Both parts of the file answer. The plane's keys are whole and need only be
/// read back and filtered; the tree is walked down to each path the
/// declarations call one value - a leaf, or an entry on a map's level - and
/// that is taken whole, however deep its own shape goes.
///
/// Only the declarations that reach into `prefix` are carried into the walk, so
/// what every node is asked is a handful of comparisons rather than the store's
/// whole schema.
pub(super) fn scan_paths_impl<D: TextDocument>(
    doc: &D,
    prefix: &StorePath,
    declared: &Declared,
) -> StorageResult<Vec<StorePath>> {
    let mut found = paths_under(doc, prefix, declared)?;

    found.sort();

    // A file can spell one path two ways - `"ui.width"` at the root and `"ui":
    // {"width": ..}` beside it - and both readings land on the same path: one
    // from the plane, one from the walk. Only one of them is reachable, because
    // `layout::node_at` resolves the path and gets whichever it finds first, so
    // handing the path back twice would report a key that answers once.
    found.dedup();

    Ok(found)
}

/// The same paths in whatever order the file gave them up.
///
/// The order a store lists in is the levels', and a document holds its levels
/// in an order of its own - the one they were written in - so the two never
/// agree and the listing has to be sorted. A caller that is going to look every
/// path up rather than list them does not need that, and the diff is two of
/// those.
fn paths_under<D: TextDocument>(
    doc: &D,
    prefix: &StorePath,
    declared: &Declared,
) -> StorageResult<Vec<StorePath>> {
    let (mut found, trees) = at_the_root(doc, declared, prefix)?;

    for at in trees {
        if !at.overlaps(prefix) {
            continue;
        }

        // Whichever of the two is deeper: a scan under a declared prefix starts
        // there, and a scan above one starts at the tree.
        let from = match prefix.starts_with(&at) {
            true => prefix.clone(),
            false => at,
        };

        walk(doc, &from, &declared.under(prefix), &mut found)?;
    }

    Ok(found)
}

/// What the file's outermost level holds, in one pass: the plane's keys that
/// fall under `prefix`, as the paths they spell, and the outermost level of
/// each tree.
///
/// One pass rather than two, and keys rather than entries. Both halves are read
/// off the same names, and neither wants the values that sit under them - which
/// on a document that owns its subtrees is a deep copy per key.
fn at_the_root<D: TextDocument>(
    doc: &D,
    declared: &Declared,
    prefix: &StorePath,
) -> StorageResult<(Vec<StorePath>, Vec<StorePath>)> {
    let mut plane = Vec::new();
    let mut trees = Vec::new();

    for key in doc.scan_keys(&StorePath::root())? {
        let (at, root) = layout::at_root(declared, &key)?;

        match root {
            layout::Root::Tree => trees.push(at),
            layout::Root::Plane if at.starts_with(prefix) => plane.push(at),
            layout::Root::Plane => {}
        }
    }

    Ok((plane, trees))
}

/// The same walk, with each path's node rendered to this codec's bytes.
pub(super) fn scan_prefix_impl<D: TextDocument>(
    doc: &D,
    prefix: &StorePath,
    declared: &Declared,
) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
    let mut results = Vec::new();

    for at in scan_paths_impl(doc, prefix, declared)? {
        let Some(node) = layout::node_at(doc, declared, &at) else {
            continue;
        };

        let bytes = D::node_to_bytes(node)
            .change_context(StorageError::Scan)
            .attach_prefix(prefix)
            .attach_key(&at)?;
        results.push((at, bytes));
    }

    Ok(results)
}

fn walk<D: TextDocument>(
    doc: &D,
    at: &StorePath,
    declared: &Declared,
    found: &mut Vec<StorePath>,
) -> StorageResult<()> {
    let below = match declared.holds(at) {
        Holds::Value => Vec::new(),
        Holds::Level => doc.scan_keys(at)?,
    };

    if below.is_empty() {
        if !at.is_root() && doc.get(at).is_some() && !holding_nothing(doc, declared, at) {
            found.push(at.clone());
        }
        return Ok(());
    }

    for key in below {
        walk(doc, &key, declared, found)?;
    }

    Ok(())
}

/// Whether `at` is a declared map's level standing with nothing in it.
///
/// The level is the map rather than a value under it, and it is kept so an open
/// with no bookkeeping beside it can tell a map somebody emptied from one that
/// was never written - see [`keep_a_declared_level`]. A scan must not hand it
/// back as a key: nothing is stored there, and no flat engine has one to list.
fn holding_nothing<D: TextDocument>(doc: &D, declared: &Declared, at: &StorePath) -> bool {
    use super::document::Navigable;

    declared.owns_level(at)
        && doc
            .get(at)
            .is_some_and(|node| node.is_map() && !node.has_children())
}

/// A level's addressable children, in the order the node holds them, or nothing
/// where the path holds a value.
///
/// A name no path can hold is left out here, the way a scan leaves it out, so
/// that a level holding only such children looks childless and
/// [`differing_under`] reports the level itself. Kept in, it would look like a
/// level with children that turn out to have no paths, which is a change
/// nobody reports and nobody descends into.
fn addressable<'a, D: TextDocument>(
    node: Option<&'a D::Node>,
    holds: &Holds,
) -> impl Iterator<Item = (&'a str, &'a D::Node)> {
    let level = match holds {
        Holds::Level => node,
        Holds::Value => None,
    };

    level
        .into_iter()
        .flat_map(|node| node.each_child())
        .filter(|(name, _)| !name.is_empty())
}

/// The same walk [`walk`] makes, made down two documents at once and stopping
/// wherever they agree.
///
/// The two documents are asked at every level, not only at the root: a level
/// that holds the same subtree on both sides has nothing under it to report,
/// and a node that can answer that in one comparison - which is what a level's
/// hash is for - turns the descent from the size of the document into the size
/// of the change. A node that cannot answer says so, and this walks it the way
/// it always did.
///
/// The two levels are walked side by side, so a child both readings hold and
/// hold the same costs one comparison rather than a path built and two lookups
/// made to get to it. A file somebody edited in place still holds its names
/// where it held them, so the order both sides arrive in is already a shared
/// one and [`side_by_side`] walks them as they come; only a level whose names
/// really did move is gathered and sorted, by [`in_one_order`].
///
/// What comes out is exactly the union of `walk(old)` and `walk(new)`, which is
/// what the two separate walks produced, so the events are the same set. The
/// leaf rule is applied per side for that reason: a path one reading holds as a
/// value and the other holds as a level is a path both of them name.
fn differing_under<D: TextDocument>(
    old: &D,
    new: &D,
    at: &StorePath,
    declared: &Declared,
    found: &mut Vec<StorePath>,
) -> StorageResult<()> {
    let held = layout::node_at(old, declared, at);
    let there = layout::node_at(new, declared, at);

    if let (Some(held), Some(there)) = (held, there)
        && held.known_same(there)
    {
        return Ok(());
    }

    let holds = declared.holds(at);
    let (mut mine, mut theirs) = (
        addressable::<D>(held, &holds).peekable(),
        addressable::<D>(there, &holds).peekable(),
    );

    let names =
        |doc: &D| !at.is_root() && doc.get(at).is_some() && !holding_nothing(doc, declared, at);

    let (childless, theirs_childless) = (mine.peek().is_none(), theirs.peek().is_none());

    if childless && names(old) {
        found.push(at.clone());
    }
    if theirs_childless && names(new) {
        found.push(at.clone());
    }

    if childless && theirs_childless {
        return Ok(());
    }

    let mut walked = Vec::new();

    if side_by_side::<D>(old, new, at, declared, mine, theirs, &mut walked)? {
        found.append(&mut walked);
        return Ok(());
    }

    in_one_order::<D>(old, new, at, declared, &holds, found)
}

/// The two levels walked as they come, for as long as they hold the same names
/// in the same order.
///
/// `false` where they stop doing so, and then nothing has been reported: what
/// was found on the way is left in `walked` for the caller to drop, because a
/// level whose names moved has to be read in an order both sides share before
/// any of it can be believed.
#[allow(clippy::too_many_arguments)]
fn side_by_side<'a, D: TextDocument>(
    old: &D,
    new: &D,
    at: &StorePath,
    declared: &Declared,
    mut mine: std::iter::Peekable<impl Iterator<Item = (&'a str, &'a D::Node)>>,
    mut theirs: std::iter::Peekable<impl Iterator<Item = (&'a str, &'a D::Node)>>,
    walked: &mut Vec<StorePath>,
) -> StorageResult<bool> {
    loop {
        match (mine.peek(), theirs.peek()) {
            (None, None) => return Ok(true),
            (Some((ours, _)), Some((beside, _))) if ours == beside => {}
            _ => return Ok(false),
        }

        let (name, under) = mine.next().expect("a name was there to peek at");
        let (_, beyond) = theirs.next().expect("a name was there to peek at");

        if under.known_same(beyond) {
            continue;
        }

        let key = at.push_shared(SmolStr::new(name));

        differing_under(old, new, &key, declared, walked)?;
    }
}

/// The two levels gathered, put in one order and merged.
///
/// What a merge needs is both sides ordered alike, and any order will do - this
/// is the one a level reaches when the order it is written in is not shared.
fn in_one_order<D: TextDocument>(
    old: &D,
    new: &D,
    at: &StorePath,
    declared: &Declared,
    holds: &Holds,
    found: &mut Vec<StorePath>,
) -> StorageResult<()> {
    let sorted = |node| {
        let mut held: Vec<_> = addressable::<D>(node, holds).collect();
        held.sort_by(|(ours, _), (theirs, _)| ours.cmp(theirs));
        held
    };

    let mine = sorted(layout::node_at(old, declared, at));
    let theirs = sorted(layout::node_at(new, declared, at));

    let (mut ours, mut beside) = (mine.iter().peekable(), theirs.iter().peekable());

    while ours.peek().is_some() || beside.peek().is_some() {
        let name: &str = match (ours.peek(), beside.peek()) {
            (Some((ours, _)), Some((theirs, _))) => ours.min(theirs),
            (Some((ours, _)), None) => ours,
            (None, Some((theirs, _))) => theirs,
            (None, None) => break,
        };

        let under = ours
            .next_if(|(held, _)| *held == name)
            .map(|(_, node)| *node);
        let beyond = beside
            .next_if(|(held, _)| *held == name)
            .map(|(_, node)| *node);

        if let (Some(under), Some(beyond)) = (under, beyond)
            && under.known_same(beyond)
        {
            continue;
        }

        let key = at.push_shared(SmolStr::new(name));

        differing_under(old, new, &key, declared, found)?;
    }

    Ok(())
}

/// Every path the two readings could disagree about, and no more.
///
/// The two documents are walked together rather than each into a map of its
/// own. A name the file's outermost level holds is either a whole key of the
/// plane or the top of a declared tree, and either way, when both readings hold
/// it and hold it the same, nothing under it can have changed - so the name is
/// passed over without a path being built for it, let alone a value read.
///
/// What is left is the size of the change plus one comparison per name, where
/// building both maps was the size of the document twice over. The comparison
/// per name is the floor: two readings of a file share no memory, so there is
/// nothing cheaper than looking at each name to find out which ones moved.
fn paths_that_differ<D: TextDocument>(
    old: &D,
    new: &D,
    declared: &Declared,
) -> StorageResult<Vec<StorePath>> {
    let root = StorePath::root();
    let (Some(before), Some(after)) = (old.get(&root), new.get(&root)) else {
        // One of them has no outermost level to read, which is not a document
        // this library wrote. Fall back to the walk that asks each on its own.
        let mut found = paths_under(old, &root, declared)?;
        found.extend(paths_under(new, &root, declared)?);
        found.sort();
        found.dedup();
        return Ok(found);
    };

    let mut names = before.child_names();
    names.extend(after.child_names());
    names.sort();
    names.dedup();

    let mut found = Vec::new();
    let under_root = declared.under(&root);

    for name in names {
        let held = before.get_child(Stored::read(&name));
        let there = after.get_child(Stored::read(&name));

        if let (Some(held), Some(there)) = (held, there)
            && held.known_same(there)
        {
            continue;
        }

        let key = StorePath::segment(&name);

        match layout::at_root(declared, &key)? {
            (at, layout::Root::Plane) => found.push(at),
            (at, layout::Root::Tree) => {
                differing_under(old, new, &at, &under_root, &mut found)?;
            }
        }
    }

    found.sort();
    found.dedup();

    Ok(found)
}

/// What changed between two readings of the data file, as events.
///
/// Reads both with the declarations this binary carries rather than the ones
/// the store recorded: an edit picked up from the file is handed to
/// subscribers here, and a subscriber is code in this process, watching the
/// paths this process declares.
pub fn diff_documents<D: TextDocument>(
    old: &D,
    new: &D,
    at: u64,
) -> StorageResult<Vec<StoreEvent>> {
    let declared = Declared::compiled_in();

    // The reading that found nothing changed is the one a watcher makes nearly
    // every time - a file is touched far more often than its contents move -
    // and it used to cost what replacing the whole document costs, because
    // nothing here looked at the size of the change before doing the work.
    if let (Some(before), Some(after)) = (old.get(&StorePath::root()), new.get(&StorePath::root()))
        && before.known_same(after)
    {
        return Ok(Vec::new());
    }

    let all_keys = paths_that_differ(old, new, declared)?;

    let mut events = Vec::new();

    for path in &all_keys {
        let old_node = layout::node_at(old, declared, path);
        let new_node = layout::node_at(new, declared, path);

        match (old_node, new_node) {
            (Some(o), Some(n)) if o.known_same(n) => {}
            (Some(o), Some(n)) => {
                let old_bytes = D::node_to_bytes(o).ok();
                let new_bytes = D::node_to_bytes(n).ok();
                if old_bytes != new_bytes {
                    events.push(StoreEvent {
                        path: path.clone(),
                        op: StoreOp::Set,
                        old: old_bytes,
                        new: new_bytes,
                        at,
                        source: Source::Disk,
                    });
                }
            }
            (Some(o), None) => {
                let old_bytes = D::node_to_bytes(o).ok();
                events.push(StoreEvent {
                    path: path.clone(),
                    op: StoreOp::Delete,
                    old: old_bytes,
                    new: None,
                    at,
                    source: Source::Disk,
                });
            }
            (None, Some(n)) => {
                let new_bytes = D::node_to_bytes(n).ok();
                events.push(StoreEvent {
                    path: path.clone(),
                    op: StoreOp::Set,
                    old: None,
                    new: new_bytes,
                    at,
                    source: Source::Disk,
                });
            }
            (None, None) => {}
        }
    }

    Ok(events)
}

impl<D: TextDocument> format::FormatRecord for TextStore<D> {
    fn format_facts(&self) -> StorageResult<Option<StorageFactSet>> {
        self.inner.read_format_facts()
    }

    fn set_format_facts(&self, facts: &StorageFactSet) -> StorageResult<()> {
        self.inner.write_format_facts(facts)
    }
}
