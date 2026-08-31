use crate::codec::CodecError;
use crate::migration::AppliedStep;
use crate::migration::set::MigrationSet;
use crate::store::error::{StorageError, StorageResult};
use crate::store::facts::{Facts, ValueBytes};
use amethystate_core::path::{IntoStorePath, StorePath};

use crate::store::meta::{PrefixMeta, SchemaSnapshot};
use crate::store::{CodecFormat, Kv, StoreCallback, SubscriptionId};
use crate::{MigrationReport, Store, SubscriptionKind};
use error_stack::{Report, ResultExt};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

/// Where a store keeps what it keeps.
///
/// A shape rather than a list, so reaching a particular file is a match and
/// not a search: an engine that has no separate bookkeeping cannot be asked
/// for it, and one that has cannot be missing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreLayout {
    /// One file holds the values and the bookkeeping together, and the engine
    /// keeps whatever else it needs inside it.
    Single { data: std::path::PathBuf },

    /// Values and bookkeeping in files of their own, each with the copy kept
    /// while it is rewritten so a rewrite that fails partway can be put back.
    Sidecars {
        data: std::path::PathBuf,
        meta: std::path::PathBuf,
        data_backup: std::path::PathBuf,
        meta_backup: std::path::PathBuf,
    },
}

/// The path a caller named, or why what they gave is not one.
///
/// Every typed entry point takes `impl IntoStorePath` and has to make the same
/// conversion; doing it here means the failure gets named once rather than at
/// each of them.
pub fn to_path(path: impl IntoStorePath) -> StorageResult<StorePath> {
    path.into_store_path().change_context(StorageError::Path)
}

/// One more level under `path`, named by a map key.
pub fn entry_path(path: &StorePath, key: impl AsRef<str>) -> StorageResult<StorePath> {
    let key = key.as_ref();
    path.try_push(key)
        .change_context(StorageError::Path)
        .attach_prefix(path)
        .attach_entry(key)
}

pub trait MigrationBackendAdapter {
    fn format(&self) -> CodecFormat;

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>>;
    fn set(&mut self, key: &str, value: &[u8]) -> StorageResult<()>;
    fn delete(&mut self, key: &str) -> StorageResult<()>;
    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>>;

    fn get_meta(&self, prefix: &StorePath) -> StorageResult<Option<PrefixMeta>>;
    fn set_meta(&mut self, prefix: &StorePath, meta: &PrefixMeta) -> StorageResult<()>;
    fn get_schema_snapshot(&self, prefix: &StorePath) -> StorageResult<Option<SchemaSnapshot>>;
    fn set_schema_snapshot(
        &mut self,
        prefix: &StorePath,
        snapshot: &SchemaSnapshot,
    ) -> StorageResult<()>;
    fn get_migration_log(&self, prefix: &StorePath) -> StorageResult<Option<Vec<AppliedStep>>>;
    fn set_migration_log(&mut self, prefix: &StorePath, log: &[AppliedStep]) -> StorageResult<()>;
}

pub trait SchemaAwareStore: StoreBackend {
    fn run_migrations(&self, mset: MigrationSet) -> StorageResult<MigrationReport>;
}

/// The store addressed by path, with nothing in the way.
///
/// These are the backrooms. Here be dragons.
///
/// [`Kv`](crate::store::Kv) is the surface to reach for: it refuses a write at a
/// path a declared struct owns, so a `u16` field cannot be overwritten with a
/// `String` by code that never saw the declaration. Nothing here does. A write
/// through this trait lands wherever it is aimed, and `delete_prefix` takes the
/// subtree it is given - declared paths included, and the initialization markers
/// that decide whether defaults are seeded left behind.
///
/// Which is the point: the engines implement it, the schema layer is built on
/// it, and a caller who knows exactly what they are addressing can use it. A
/// caller who is guessing wants `Kv`.
pub trait StoreBackend: Send + Sync + 'static {
    fn get_raw(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>>;

    fn set_erased(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<Uuid>,
    ) -> StorageResult<()>;

    fn set_owned_erased(
        &self,
        path: StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<Uuid>,
    ) -> StorageResult<()>;

    /// Runs `f` against a deserializer positioned at `path`, in the backend's
    /// own format. `Ok(false)` means the key is absent and `f` never ran.
    fn get_erased(
        &self,
        path: &StorePath,
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<bool>;

    /// Same, for bytes carried by a [`crate::StoreEvent`].
    fn decode_erased(
        &self,
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()>;

    fn delete_with_source(&self, path: &StorePath, source: Option<Uuid>) -> StorageResult<()>;
    fn delete(&self, path: &StorePath) -> StorageResult<()>;

    /// Removes every key under `prefix`, emitting one
    /// [`crate::StoreOp::DeletePrefix`] instead of a `Delete` per key.
    fn delete_prefix_with_source(
        &self,
        prefix: &StorePath,
        source: Option<Uuid>,
    ) -> StorageResult<()>;

    fn delete_prefix(&self, prefix: &StorePath) -> StorageResult<()> {
        self.delete_prefix_with_source(prefix, None)
    }

    /// Every key under `prefix`, sorted by key on every backend.
    ///
    /// Lists what [`StoreBackend::scan_keys`] lists.
    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>>;

    /// Hands every entry under `prefix` to `visit`, in the order a scan lists.
    ///
    /// What it saves over [`StoreBackend::scan_prefix`] is everything that has
    /// to be built to hand an entry over as owned: a `StorePath` per key,
    /// which is a string and a walk, and a `Vec` per value, which is a copy
    /// out of the engine's page. A caller that decodes each entry on the spot
    /// - which is what loading a map is - drops both immediately.
    ///
    /// The key arrives as it is stored, joined and escaped, and has not been
    /// checked: [`level_under`](amethystate_core::path::level_under) reads a
    /// level out of one and refuses a key this library did not write.
    ///
    /// Defaulted through `scan_prefix`, so a backend implemented outside this
    /// crate stays correct without knowing this exists.
    fn visit_prefix(
        &self,
        prefix: &StorePath,
        visit: &mut dyn FnMut(&str, &[u8]) -> StorageResult<()>,
    ) -> StorageResult<()> {
        for (path, bytes) in self.scan_prefix(prefix)? {
            visit(path.as_str(), &bytes)?;
        }
        Ok(())
    }

    /// The keys under `prefix`, sorted, without reading their values.
    ///
    /// `scan_prefix` copies every value out of the backend, which is wasted
    /// work when only the keys are wanted - and grows with the data rather
    /// than with the answer.
    ///
    #[doc = include_str!("scan_contract.md")]
    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>>;

    /// Whether this store was asked to read large collections on more than one
    /// core - [`StoreConfig::parallel_reads`](crate::store::config::StoreConfig).
    ///
    /// Defaulted so a backend implemented outside this crate need not know the
    /// question exists; answering `false` only means its reads stay on the
    /// calling thread, which is what they did before the question was asked.
    fn parallel_reads(&self) -> bool {
        false
    }

    /// Where this store keeps what it keeps.
    ///
    /// A caller that has to reach a store's files - a backup tool, an
    /// uninstaller, a test - would otherwise rebuild their names from the one
    /// it was given, which means writing down a rule the engine owns. The
    /// engine says it instead.
    ///
    /// Paths, not contents: a file is named whether or not it exists right
    /// now, because a backup exists only while a rewrite is in flight and its
    /// name is wanted either way.
    ///
    /// `None` for a backend implemented outside this crate, which need not
    /// answer.
    fn files(&self) -> Option<StoreLayout> {
        None
    }

    /// Writes everything buffered and says whether it landed.
    ///
    /// This is the fallible half of dropping the store, and the point of
    /// calling it is the `Result`: a full disk, a locked file or a permission
    /// error at exit is answered here, while the application is still running
    /// and can do something about it. Left to `Drop`, the same failure can
    /// only be logged.
    ///
    /// The store goes on working afterwards, and a call with nothing buffered
    /// does nothing. The file is released when the last clone of the store
    /// goes.
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc_save_now");
    /// let store = StoreBuilder::new(&*path).build().unwrap();
    /// store.kv().set("port", &8080u16).unwrap();
    ///
    /// if let Err(report) = store.save_now() {
    ///     eprintln!("settings were not saved: {report:?}");
    /// }
    /// ```
    fn save_now(&self) -> StorageResult<()>;

    /// Writes what is buffered, stops the background threads and lets go of
    /// the file.
    ///
    /// This is what hands the file to somebody else - another process, a
    /// backup, a rename. Afterwards every read and write answers
    /// [`StorageError::Closed`](crate::store::StorageError::Closed) rather
    /// than opening it again, because taking it back would leave two owners
    /// each believing they hold it. Values already read stay readable in
    /// memory; it is the store that is closed, not the handles onto it.
    ///
    /// It closes for every clone, since there is one file between them.
    /// Calling it more than once is fine, and `Drop` after it does nothing.
    ///
    /// What each engine gives up differs: sqlite releases the file itself,
    /// redb releases its claim so another store can open it, and a document
    /// engine holds nothing open and only settles its threads.
    ///
    /// The default writes what is buffered and leaves the store working, which
    /// is the whole of closing for a backend implemented outside this crate
    /// that holds nothing to give up. One that does hold something - a file
    /// handle, a connection, a thread - implements this and
    /// [`StoreBackend::is_closed`] together.
    fn close(&self) -> StorageResult<()> {
        self.save_now()
    }

    /// Whether [`StoreBackend::close`] has already run.
    ///
    /// For a reader that holds a value of its own and wants to say where it
    /// came from: a closed store reports nothing further, so what such a
    /// reader holds is the last thing it was told.
    fn is_closed(&self) -> bool {
        false
    }

    fn subscribe(&self, kind: SubscriptionKind, callback: StoreCallback) -> SubscriptionId;
    fn unsubscribe(&self, id: SubscriptionId);

    /// Flushes pending in-memory modifications under the specified prefix to disk.
    ///
    /// # Note
    /// Behavior is backend-specific: transactional engines (such as `redb`, `sqlite`) will
    /// selectively commit changes under the given prefix, while monolithic document engines
    /// (such as `json`, `toml`) will serialize and rewrite the entire file.
    fn flush_prefix(&self, prefix: &StorePath) -> StorageResult<()>;

    /// Commits without blocking; the future resolves once a flush has landed.
    ///
    /// Waiters ride on the flush the store was going to do anyway, so several
    /// of them cost one commit rather than one each.
    fn flush_async(&self) -> crate::store::durable::Commit;

    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool>;

    /// Records whether `namespace` has been seeded.
    ///
    /// The one bit no amount of reading the data reproduces: a namespace whose
    /// values were all removed looks exactly like one that was never written.
    /// Which way it reads decides whether the next construction puts the
    /// declared defaults back.
    ///
    /// Setting a namespace [`Fresh`](InitState::Fresh) that was never seeded is
    /// not an error.
    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()>;

    fn mark_initialized(&self, namespace: &StorePath) -> StorageResult<()> {
        self.set_initialized(namespace, InitState::Seeded)
    }
}

/// Whether a namespace has had its declared defaults written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitState {
    /// The defaults have been written; do not write them again.
    Seeded,

    /// Nothing has been written here, so the next construction seeds it.
    Fresh,
}

impl InitState {
    pub fn is_seeded(self) -> bool {
        matches!(self, InitState::Seeded)
    }
}

/// The typed surface over [`StoreBackend`]. Blanket-implemented, including for
/// `dyn StoreBackend`, so a call site never has to know which it holds.
pub trait StoreExt: StoreBackend {
    fn get<T: DeserializeOwned>(&self, path: impl IntoStorePath) -> StorageResult<Option<T>> {
        let path = to_path(path)?;
        let mut out = None;
        let found = self.get_erased(&path, &mut |d| {
            out = Some(
                erased_serde::deserialize::<T>(d)
                    .map_err(CodecError::from)
                    .change_context(StorageError::Codec)
                    .attach_key(&path)?,
            );
            Ok(())
        })?;
        Ok(if found { out } else { None })
    }

    fn set<T: Serialize>(&self, path: impl IntoStorePath, value: &T) -> StorageResult<()> {
        self.set_erased(&to_path(path)?, &value, None)
    }

    fn set_owned<T: Serialize>(&self, path: StorePath, value: &T) -> StorageResult<()> {
        self.set_owned_erased(path, &value, None)
    }

    fn set_with_source<T: Serialize>(
        &self,
        path: impl IntoStorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.set_erased(&to_path(path)?, &value, source)
    }

    fn set_owned_with_source<T: Serialize>(
        &self,
        path: StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.set_owned_erased(path, &value, source)
    }

    /// Reads bytes that arrived in a [`StoreEvent`](crate::StoreEvent) as `T`.
    fn decode<T: DeserializeOwned>(&self, bytes: &[u8]) -> StorageResult<T> {
        let mut out = None;
        self.decode_erased(bytes, &mut |d| {
            out = Some(
                erased_serde::deserialize::<T>(d)
                    .map_err(CodecError::from)
                    .change_context(StorageError::Codec)
                    .attach_with(|| format!("as: {}", std::any::type_name::<T>()))?,
            );
            Ok(())
        })?;

        out.ok_or_else(|| {
            Report::new(StorageError::Codec)
                .attach("the backend accepted the bytes without producing a value")
                .attach(ValueBytes(bytes.len()))
        })
    }
}

impl<S: StoreBackend + ?Sized> StoreExt for S {}

/// Reactive values addressed by path, without declaring a struct. See [`crate::store::Kv`].
impl Store {
    pub fn kv(&self) -> Kv {
        Kv::new(self.clone())
    }
}
