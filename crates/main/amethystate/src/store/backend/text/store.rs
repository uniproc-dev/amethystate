use super::document::TextDocument;
use super::error::TextStoreError;
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
use crate::store::config::{FileWritePolicy, StoreConfig};
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
    SubscriptionEntry, SubscriptionId, SubscriptionKind,
};
use amethystate_core::Source;
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::NamedTempFile;
use tracing::{error, info, warn};

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

/// Where a record named `key` sits in the metadata file, which is one level
/// deep: the whole key is the name.
///
/// The one place the joining is undone into a name again, and the reason
/// [`meta_key`] can go on being a path - a report names the record it is about,
/// and the file holds it whole.
pub(super) fn meta_at(key: &StorePath) -> StorePath {
    StorePath::segment(key.as_str())
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

    let named = StorePath::parse_joined(&name)
        .change_context(StorageError::Path)
        .attach_key(key)?;

    Ok(named.strip_prefix(&StorePath::segment(kind)))
}

impl<D: TextDocument> StoreFile<D> {
    pub fn new(path: PathBuf, initial_doc: D, write_policy: FileWritePolicy) -> Self {
        let backup_path = StoreLayout::rewrite_copy_of(&path);
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

    /// Reads the file, recovering from the copy beside it where it will not
    /// read.
    ///
    /// Nothing is copied here. The copy is taken once the whole open is known
    /// to go ahead - see [`StoreFiles::load_and_back_up`] - because an open
    /// that is refused must leave nothing of its own behind: a `.bak` beside
    /// the store is read by the next open as an unfinished previous run, and
    /// it would recover onto it.
    pub fn read_or_recover(&self) -> StorageResult<D> {
        self.read_or_recover_unless(|_| None)
    }

    /// The same, with a second reason a document may be no good.
    ///
    /// A parse failure is not the only way a file arrives half-written: a
    /// format that calls an empty file a valid empty document parses it
    /// happily, and only something outside the file knows better. `suspect`
    /// says so, and what it names is treated exactly as a parse failure -
    /// recovered from the backup, and refused if there is none.
    pub fn read_or_recover_unless(
        &self,
        suspect: impl Fn(&D) -> Option<&'static str>,
    ) -> StorageResult<D> {
        let read = match self.lost_its_file() {
            true => Err(error_stack::Report::new(StorageError::Open)
                .attach(StoreFileFact(self.path.clone()))
                .attach("the file is gone and the copy kept for it is not")),
            false => self.load_or_empty().and_then(|doc| match suspect(&doc) {
                Some(why) => Err(error_stack::Report::new(StorageError::Open)
                    .attach(StoreFileFact(self.path.clone()))
                    .attach(why)),
                None => Ok(doc),
            }),
        };

        match read {
            Ok(doc) => Ok(doc),
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

    /// Whether the file is absent while the copy kept for it is not.
    ///
    /// An absent file is an unwritten store, which is the ordinary first open -
    /// unless a backup stands beside it, and then it is a store whose file went
    /// missing since the last open wrote one. That is the case the copy exists
    /// for, and reading it as a new store loses both: the empty document is
    /// persisted over the data file and the copy is cleaned up behind it.
    fn lost_its_file(&self) -> bool {
        !self.path.exists() && self.backup_path.exists()
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
    /// two flushes would cross: A renders, B renders, B replaces, A replaces,
    /// and the file ends up holding what A saw.
    pub fn persist(&self) -> StorageResult<()> {
        let _flushing = self.flush.lock();

        let content = self.doc.read().serialize().attach_store_file(&self.path)?;
        persist_atomic(&self.path, &content, self.write_policy)
            .map_err(TextStoreError::from)
            .change_context(StorageError::Flush)
            .attach_store_file(&self.path)?;
        Ok(())
    }

    /// Puts the file back the way this open found it, and says so when it
    /// cannot.
    ///
    /// Nothing is returned because the caller is already carrying the failure
    /// that brought it here, and there is no answer to a restore that will not
    /// land. There is a report, though: this is the one path that leaves the
    /// file holding what a half-finished open wrote.
    pub fn restore_from_backup(&self, fallback_to_initial: &D) {
        *self.doc.write() = fallback_to_initial.clone();

        if self.backup_path.exists() {
            if let Err(io) = std::fs::copy(&self.backup_path, &self.path) {
                error!(
                    file = %self.path.display(),
                    backup = %self.backup_path.display(),
                    error = %io,
                    "the copy taken at the start of this open could not be put back, so the \
                     file holds what the open that failed had written"
                );
                return;
            }

            self.remove_backup("after putting it back");
        } else if self.path.exists()
            && let Err(io) = std::fs::remove_file(&self.path)
        {
            error!(
                file = %self.path.display(),
                error = %io,
                "this open created the file and could not take it away again, so a store \
                 that was never opened is left on disk"
            );
        }
    }

    pub fn clean_backup(&self) {
        if self.backup_path.exists() {
            self.remove_backup("after the open went through");
        }
    }

    /// A copy left behind is read by the next open as an unfinished previous
    /// run, so failing to take one away is worth a line.
    fn remove_backup(&self, when: &'static str) {
        if let Err(io) = std::fs::remove_file(&self.backup_path) {
            warn!(
                file = %self.path.display(),
                backup = %self.backup_path.display(),
                error = %io,
                "the copy beside the store could not be removed {when}, and the next open \
                 reads one as an unfinished previous run"
            );
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

/// Where the metadata records that the data file held something.
///
/// The one fact that tells a store somebody emptied from a file caught
/// half-written: both are a document with no keys, and the file cannot say
/// which it is. TOML shows it plainest - an empty file is a valid empty
/// document - but the question is not toml's, and neither is the answer.
///
/// It lives in the metadata because it is the store's opinion of itself.
/// Writing it into the data file would put it where a person edits, where they
/// would rightly delete it, and where a store that was cleared would stop being
/// empty.
fn held_key() -> StorePath {
    meta_at(&meta_key("held", &StorePath::root()))
}

/// Whether a document holds no key at all, which is what a store somebody
/// emptied and a file caught half-written both look like.
///
/// A document that will not even be scanned is not called empty: the question
/// here is only whether it is, and anything else is somebody else's failure to
/// report.
pub(super) fn has_no_keys<D: TextDocument>(doc: &D) -> bool {
    doc.scan(&StorePath::root())
        .map(|children| children.is_empty())
        .unwrap_or(false)
}

/// Puts back a declared map's level where deleting its last entry took it.
///
/// A document drops a level nothing is left in, which is right for a level that
/// existed only to hold what was deleted. A declared map is not that: its level
/// is the map, and whether it stands is how a store whose bookkeeping went
/// missing tells a map somebody emptied from one that was never written at all.
/// See [`Declared::owns_level`].
fn keep_a_declared_level<D: TextDocument>(
    doc: &mut D,
    declared: &Declared,
    at: &StorePath,
) -> StorageResult<()> {
    let Some(level) = at.parent() else {
        return Ok(());
    };

    if !declared.owns_level(&level) || doc.get(&level).is_some() {
        return Ok(());
    }

    doc.set(
        &level,
        <D::Node as super::document::Navigable>::make_empty_map(),
    )
}

impl<D: TextDocument> StoreFiles<D> {
    /// Both documents, and the data one checked against what the metadata
    /// remembers of it.
    ///
    /// The metadata is read first because it is what judges the data: a
    /// document with no keys where the last save had some is a file that was
    /// truncated between then and now, and taking it at face value would save
    /// the emptiness back over everything.
    ///
    /// Both copies are taken at the end, once both files have read, and not
    /// one of them before. An open that is refused is an operation that did
    /// not happen, and it leaves nothing of its own: a `.bak` beside the store
    /// is read by the next open as an unfinished previous run, and it would
    /// recover onto it.
    pub fn load_and_back_up(&self) -> StorageResult<(D, D)> {
        // Read before the data and reported after it: what it says is needed to
        // judge the data, and a metadata file that will not read is no reason
        // for the data to go unread.
        let meta = self.meta.read_or_recover();

        let held = matches!(
            meta.as_ref()
                .ok()
                .and_then(|held: &D| held.get(&held_key()))
                .map(D::deserialize_node::<bool>),
            Some(Ok(true))
        );

        let data = self
            .data
            .read_or_recover_unless(|doc: &D| match held && has_no_keys(doc) {
                true => Some("the last save left keys here and the file now holds none"),
                false => None,
            })
            .attach("role: the store's data")?;

        let meta = meta.attach("role: the store's schema bookkeeping")?;

        self.data.create_backup().attach("role: the store's data")?;
        self.meta
            .create_backup()
            .attach("role: the store's schema bookkeeping")?;

        Ok((data, meta))
    }

    /// Writes the metadata first, and the fact about the data before the data.
    ///
    /// A crash between the two leaves the metadata saying more than the file
    /// does, which costs a refusal the backup answers. The other order costs
    /// the data: an emptied store whose metadata still says it held keys reads
    /// as truncated for good.
    pub fn persist(&self) -> StorageResult<()> {
        self.remember_what_the_data_holds()?;

        self.meta
            .persist()
            .attach("role: the store's schema bookkeeping")?;
        self.data.persist().attach("role: the store's data")?;
        Ok(())
    }

    fn remember_what_the_data_holds(&self) -> StorageResult<()> {
        let holds = !has_no_keys(&*self.data.doc.read());
        let key = held_key();

        {
            let mut guard = self.meta.doc.write();
            let said = matches!(
                guard.get(&key).map(D::deserialize_node::<bool>),
                Some(Ok(true))
            );

            if said == holds {
                return Ok(());
            }

            let node = D::serialize_node(&holds, &Noticed::unlimited())
                .change_context(StorageError::Meta)
                .attach_key(&key)?;

            guard
                .set(&key, node)
                .change_context(StorageError::Meta)
                .attach_key(&key)?;
        }

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

        let (initial_data, initial_meta) = match files.load_and_back_up() {
            Ok(read) => read,
            Err(why)
                if super::super::utils::start_fresh(
                    &config,
                    crate::store::builder::Backend::writing(D::format()),
                    &why,
                ) =>
            {
                files.load_and_back_up()?
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

        let files_debounce = files.clone();
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

        let settling = watching::Coalescing::new(config.watch_debounce);
        let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            let Ok(event) = res else { return };

            let is_modify = matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_));
            if !is_modify {
                return;
            }

            settling.settle();

            watching::take_outside_edit::<D>(
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
            declared: RwLock::new(None),
            bookkeeping_is_lost,
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
        refuse_closing_from_a_flush()?;
        {
            let _data = self.files.data.doc.write();
            let _meta = self.files.meta.doc.write();
            if !self.debouncer.stop_accepting() {
                return Ok(());
            }
        }

        self.debouncer.shutdown();
        self.save_now()
            .attach("rendering the document before close")
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

        let old_bytes = {
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
            keep_a_declared_level(&mut *guard, &declared, &at)
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
                source: source.into(),
            },
        )?;

        self.debouncer.schedule();
        Ok(())
    }

    fn delete_prefix(&self, prefix: &StorePath, source: Option<uuid::Uuid>) -> StorageResult<()> {
        self.check_debouncer()?;

        self.pull_external_changes();

        let declared = self.declared()?;
        {
            let mut guard = self.files.data.doc.write();
            self.refuse_if_closed()?;

            for at in plane_under(&*guard, &declared, prefix)? {
                guard
                    .delete(&StorePath::segment(at.as_str()))
                    .doing(StorageError::Delete, &self.files.data.path)
                    .attach_key(&at)?;
            }

            guard
                .delete_subtree(prefix)
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
                source: source.into(),
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
                    .attach_meta_node(key.as_str())?,
                None => Vec::new(),
            };

            match crate::store::moved::same_declaration_stored(&held, &schema.fields) {
                Some(at) if held[at] == *schema => return Ok(()),
                Some(at) => held[at] = schema.clone(),
                None => held.push(schema.clone()),
            }

            let node = D::serialize_node(&held, &Noticed::unlimited())
                .in_meta(StorageError::Meta, &self.files.meta.path)
                .attach_meta_node(key.as_str())?;

            guard
                .set(&parts, node)
                .in_meta(StorageError::Meta, &self.files.meta.path)
                .attach_meta_node(key.as_str())?;
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
        let (old_bytes, new_bytes) = {
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

            self.writes.fetch_add(1, Ordering::Release);
            (old, new)
        };

        let event = match new_bytes {
            Some(new) => StoreEvent {
                path: path_str.clone(),
                op: StoreOp::Set,
                old: old_bytes,
                new: Some(new),
                source: source.into(),
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
    let mut found = plane_under(doc, declared, prefix)?;

    for at in tree_roots(doc, declared)? {
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

    found.sort();
    Ok(found)
}

/// The plane's keys under `prefix`, as the paths they spell.
fn plane_under<D: TextDocument>(
    doc: &D,
    declared: &Declared,
    prefix: &StorePath,
) -> StorageResult<Vec<StorePath>> {
    let mut found = Vec::new();

    for (key, _) in doc.scan(&StorePath::root())? {
        let (at, root) = layout::at_root(declared, &key)?;
        if root == layout::Root::Plane && at.starts_with(prefix) {
            found.push(at);
        }
    }

    Ok(found)
}

/// The outermost level of each tree the file holds.
fn tree_roots<D: TextDocument>(doc: &D, declared: &Declared) -> StorageResult<Vec<StorePath>> {
    let mut found = Vec::new();

    for (key, _) in doc.scan(&StorePath::root())? {
        let (at, root) = layout::at_root(declared, &key)?;
        if root == layout::Root::Tree {
            found.push(at);
        }
    }

    Ok(found)
}

/// The same walk, with each path's node rendered to this codec's bytes.
pub(super) fn scan_prefix_impl<D: TextDocument>(
    doc: &D,
    prefix: &StorePath,
    declared: &Declared,
) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
    let mut results = Vec::new();

    for at in scan_paths_impl(doc, prefix, declared)? {
        let Some(node) = at_node(doc, declared, &at) else {
            continue;
        };

        let bytes = D::node_to_bytes(&node)
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
        Holds::Level => doc.scan(at)?,
    };

    if below.is_empty() {
        if !at.is_root() && doc.get(at).is_some() && !holding_nothing(doc, declared, at) {
            found.push(at.clone());
        }
        return Ok(());
    }

    for (key, _) in below {
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

fn at_node<D: TextDocument>(doc: &D, declared: &Declared, at: &StorePath) -> Option<D::Node> {
    doc.get(&layout::levels(doc, declared, at)).cloned()
}

/// What changed between two readings of the data file, as events.
///
/// Reads both with the declarations this binary carries rather than the ones
/// the store recorded: an edit picked up from the file is handed to
/// subscribers here, and a subscriber is code in this process, watching the
/// paths this process declares.
pub(super) fn diff_documents<D: TextDocument>(old: &D, new: &D) -> StorageResult<Vec<StoreEvent>> {
    let declared = Declared::compiled_in();

    let old_map = as_map(old, declared).attach("reading the document as it was before the edit")?;
    let new_map = as_map(new, declared).attach("reading the document as it is on disk")?;

    let mut events = Vec::new();

    let mut all_keys: std::collections::BTreeSet<StorePath> = old_map.keys().cloned().collect();
    all_keys.extend(new_map.keys().cloned());

    for path in all_keys {
        let old_node = old_map.get(&path);
        let new_node = new_map.get(&path);

        match (old_node, new_node) {
            (Some(o), Some(n)) => {
                let old_bytes = D::node_to_bytes(o).ok();
                let new_bytes = D::node_to_bytes(n).ok();
                if old_bytes != new_bytes {
                    events.push(StoreEvent {
                        path,
                        op: StoreOp::Set,
                        old: old_bytes,
                        new: new_bytes,
                        source: Source::Disk,
                    });
                }
            }
            (Some(o), None) => {
                let old_bytes = D::node_to_bytes(o).ok();
                events.push(StoreEvent {
                    path,
                    op: StoreOp::Delete,
                    old: old_bytes,
                    new: None,
                    source: Source::Disk,
                });
            }
            (None, Some(n)) => {
                let new_bytes = D::node_to_bytes(n).ok();
                events.push(StoreEvent {
                    path,
                    op: StoreOp::Set,
                    old: None,
                    new: new_bytes,
                    source: Source::Disk,
                });
            }
            (None, None) => {}
        }
    }

    Ok(events)
}

fn as_map<D: TextDocument>(
    doc: &D,
    declared: &Declared,
) -> StorageResult<HashMap<StorePath, D::Node>> {
    let mut found = HashMap::new();

    for at in scan_paths_impl(doc, &StorePath::root(), declared)? {
        if let Some(node) = at_node(doc, declared, &at) {
            found.insert(at, node);
        }
    }

    Ok(found)
}

impl<D: TextDocument> format::FormatRecord for TextStore<D> {
    fn format_facts(&self) -> StorageResult<Option<StorageFactSet>> {
        self.inner.read_format_facts()
    }

    fn set_format_facts(&self, facts: &StorageFactSet) -> StorageResult<()> {
        self.inner.write_format_facts(facts)
    }
}
