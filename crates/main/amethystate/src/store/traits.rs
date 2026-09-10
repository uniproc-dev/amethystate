use crate::codec::CodecError;
use crate::migration::AppliedStep;
use crate::migration::set::MigrationSet;
use crate::store::builder::Backend;
use crate::store::error::{StorageError, StorageResult};
use crate::store::facts::{Facts, ValueBytes};
use crate::store::reading::{ReadResult, ReadValue};
use amethystate_core::path::{IntoStorePath, PathRef, StorePath, StorePathError};
use amethystate_core::primitives::error::WriteValue;
use std::fmt;
use std::path::{Path, PathBuf};

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
    Single { data: PathBuf },

    /// Values and bookkeeping in files of their own, each with the copy kept
    /// while it is rewritten so a rewrite that fails partway can be put back.
    Sidecars {
        data: PathBuf,
        meta: PathBuf,
        data_backup: PathBuf,
        meta_backup: PathBuf,
    },
}

impl StoreLayout {
    /// The files a store at `path` under `backend` is made of, worked out
    /// without opening anything.
    ///
    /// The same names an open store answers with. The extension is added the
    /// way [`StoreBuilder::new`] adds it: only where the caller named none.
    ///
    /// [`StoreBuilder::new`]: crate::store::builder::StoreBuilder::new
    pub fn of(path: impl AsRef<Path>, backend: Backend) -> Self {
        let mut path = path.as_ref().to_path_buf();
        if path.extension().is_none() {
            path.set_extension(backend.extension());
        }

        match backend {
            #[cfg(feature = "redb")]
            Backend::Redb => Self::Single { data: path },
            #[cfg(feature = "sqlite")]
            Backend::Sqlite => Self::Single { data: path },
            #[cfg(feature = "json")]
            Backend::Json => Self::sidecars(path),
            #[cfg(feature = "toml")]
            Backend::Toml => Self::sidecars(path),
            #[cfg(feature = "ron")]
            Backend::Ron => Self::sidecars(path),
        }
    }

    /// The copy an engine keeps beside a file while it rewrites it.
    pub fn rewrite_copy_of(file: &Path) -> PathBuf {
        match file.file_name() {
            Some(name) => {
                let mut name = name.to_os_string();
                name.push(".bak");
                file.with_file_name(name)
            }
            None => file.with_extension("bak"),
        }
    }

    /// Every name this store uses, whether or not the file is there.
    pub fn names(&self) -> Vec<PathBuf> {
        match self {
            Self::Single { data } => vec![data.clone()],
            Self::Sidecars {
                data,
                meta,
                data_backup,
                meta_backup,
            } => vec![
                data.clone(),
                meta.clone(),
                data_backup.clone(),
                meta_backup.clone(),
            ],
        }
    }

    /// The ones that are on disk at this moment.
    ///
    /// A rewrite copy is written when a rewrite starts and removed when it
    /// finishes, so a store sitting still has none.
    pub fn present(&self) -> Vec<PathBuf> {
        self.names()
            .into_iter()
            .filter(|file| file.exists())
            .collect()
    }

    #[cfg_attr(
        not(any(feature = "json", feature = "toml", feature = "ron")),
        allow(dead_code)
    )]
    fn sidecars(data: PathBuf) -> Self {
        let meta = data.with_extension("meta");

        Self::Sidecars {
            data_backup: Self::rewrite_copy_of(&data),
            meta_backup: Self::rewrite_copy_of(&meta),
            data,
            meta,
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn the_two_copies_of_a_sidecar_store_are_two_files() {
        let layout = StoreLayout::sidecars(PathBuf::from("app/settings.db"));

        let StoreLayout::Sidecars {
            data,
            meta,
            data_backup,
            meta_backup,
        } = layout
        else {
            unreachable!("built as sidecars")
        };

        assert_eq!(data_backup, PathBuf::from("app/settings.db.bak"));
        assert_eq!(meta_backup, PathBuf::from("app/settings.meta.bak"));

        assert_ne!(
            data_backup, meta_backup,
            "a copy that keeps only the extension names one file for both, so the second \
             lands on the first and the data is left with the metadata's bytes"
        );
        assert!(
            ![&data, &meta].contains(&&data_backup) && ![&data, &meta].contains(&&meta_backup),
            "a copy took the name of a file the store writes"
        );
    }

    #[test]
    fn a_copy_keeps_the_whole_name_it_was_made_from() {
        assert_eq!(
            StoreLayout::rewrite_copy_of(Path::new("settings.conf")),
            PathBuf::from("settings.conf.bak"),
            "an extension the caller spelled is theirs, and the copy is beside it rather \
             than over a neighbour's `settings.bak`"
        );
    }
}

/// The path a caller named, or why what they gave is not one.
///
/// Every typed entry point takes `impl IntoStorePath` and has to make the same
/// conversion; doing it here means the failure gets named once rather than at
/// each of them.
pub fn to_path(path: impl IntoStorePath) -> Result<StorePath, StorePathError> {
    path.into_store_path()
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

    fn get(&self, key: &StorePath) -> StorageResult<Option<Vec<u8>>>;
    fn set(&mut self, key: &StorePath, value: &[u8]) -> StorageResult<()>;
    fn delete(&mut self, key: &StorePath) -> StorageResult<()>;
    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>>;

    /// Whether the store's own bookkeeping is gone while its data is not.
    ///
    /// A text store keeps it in a second file, which can be deleted or lost on
    /// its own; the flat engines keep it in the same transaction as the data,
    /// where the two cannot come apart, and answer `false`.
    ///
    /// What it costs is a migration: the version a prefix stands at lives in
    /// that bookkeeping, and without it there is no telling which steps have
    /// already run over the keys that are still there.
    fn bookkeeping_is_lost(&self) -> bool {
        false
    }

    /// Removes the place and everything under it.
    ///
    /// What a declaration owns is not always one key: a map owns every entry
    /// under it, and only a document engine keeps those inside a node that goes
    /// when the node does. Taken key by key here, so the engines answer a
    /// withdrawn map the same way.
    fn delete_prefix(&mut self, prefix: &StorePath) -> StorageResult<()> {
        let under = self.scan_prefix(prefix)?;
        for (path, _) in under {
            self.delete(&path)?;
        }
        self.delete(prefix)
    }

    fn get_meta(&self, prefix: &StorePath) -> StorageResult<Option<PrefixMeta>>;
    fn set_meta(&mut self, prefix: &StorePath, meta: &PrefixMeta) -> StorageResult<()>;
    /// The trees recorded at `prefix`, one per declaration.
    ///
    /// A prefix is not a place and nothing claims it, so more than one
    /// declaration may sit at one as long as their places stay apart - and
    /// each is recorded whole, because a declaration is identified by the
    /// places it owns and folding two together would lose which belonged to
    /// which. See `RFC-the-ownership-tree.md`.
    fn get_schema_snapshots(&self, prefix: &StorePath) -> StorageResult<Vec<SchemaSnapshot>>;

    fn set_schema_snapshots(
        &mut self,
        prefix: &StorePath,
        trees: &[SchemaSnapshot],
    ) -> StorageResult<()>;
    fn get_migration_log(&self, prefix: &StorePath) -> StorageResult<Option<Vec<AppliedStep>>>;
    fn set_migration_log(&mut self, prefix: &StorePath, log: &[AppliedStep]) -> StorageResult<()>;
}

pub trait SchemaAwareStore: StoreBackend {
    fn run_migrations(&self, mset: MigrationSet) -> StorageResult<MigrationReport>;
}

/// The store addressed by path, with nothing in the way.
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
    /// A key an engine holds as a string is checked on its way in, by
    /// [`PathRef::parse`], so what reaches the visitor is a path and nothing
    /// above this trait has to know a key was ever spelled. Borrowed rather
    /// than owned, because the saving is the whole reason this exists: an
    /// owned path would allocate once per entry, which on a map is once per
    /// entry of the map.
    ///
    /// Defaulted through `scan_prefix`, so a backend implemented outside this
    /// crate stays correct without knowing this exists.
    fn visit_prefix(
        &self,
        prefix: &StorePath,
        visit: &mut dyn FnMut(PathRef<'_>, &[u8]) -> StorageResult<()>,
    ) -> StorageResult<()> {
        for (path, bytes) in self.scan_prefix(prefix)? {
            visit(PathRef::from(&path), &bytes)?;
        }
        Ok(())
    }

    /// The keys under `prefix`, sorted, without reading their values.
    ///
    /// `scan_prefix` copies every value out of the backend, which is wasted
    /// work when only the keys are wanted - and grows with the data rather
    /// than with the answer.
    ///
    /// Every engine lists the same keys, and
    /// `tests/a_scan_says_the_same_on_every_engine.rs` is where that is stated
    /// rather than described. The one thing an engine can answer alone is a
    /// name no path can hold: a document may hold one, where a flat engine
    /// cannot, and the scan passes over it with a line at `warn`. Its value
    /// stays in the file and survives a save; nothing addresses it.
    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>>;

    /// Whether this store was asked to read large collections on more than one
    /// core - [`StoreConfig::parallel_reads`](crate::store::config::StoreConfig).
    ///
    /// Defaulted so a backend implemented outside this crate need not know the
    /// question exists; answering `false` only means its reads stay on the
    /// calling thread. Of the engines here, `redb` is the one that overrides
    /// it.
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
    /// **This describes a store nobody is writing to.** sqlite opens in WAL
    /// mode, so a live store holds committed data in a `-wal` sidecar until a
    /// checkpoint moves it across, and closing the store is what checkpoints.
    /// Copying the named files out from under a running store therefore takes
    /// a database missing its most recent commits - which is the condition a
    /// backup wants settled anyway, and the reason the sidecars are not named
    /// here: they belong to one engine and exist only while it is open.
    ///
    /// `None` for a backend implemented outside this crate, which need not
    /// answer.
    fn files_layout(&self) -> Option<StoreLayout> {
        None
    }

    /// The format record, for a test that wants to see or forge one.
    ///
    /// Absent from a build without `test-utils`: the write half can make a
    /// store unopenable, and an open is its only honest caller. What the open
    /// uses is `format::FormatRecord`, which is crate-private.
    #[cfg(feature = "test-utils")]
    fn format_record(&self) -> Option<&dyn crate::store::format::TestFormatRecord> {
        None
    }

    /// Reads the file back now, as the watcher would, and tells subscribers
    /// what changed.
    ///
    /// A test that edits the file underneath a running store otherwise has to
    /// wait for the operating system to notice, which is the one thing
    /// `watcher_wiring` is for. Everything else about a reread - which paths it
    /// reports, and as what - is settled here instead, without a deadline.
    ///
    /// An engine that holds no file has nothing to reread, which is why this is
    /// defaulted rather than demanded.
    #[cfg(feature = "test-utils")]
    fn reread_from_disk(&self) {}

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

    /// Gets what is buffered under `prefix` onto disk.
    ///
    /// How much else goes with it is the engine's own answer - see
    /// [`Backend::a_commit_covers_the_whole_store`](crate::store::builder::Backend::a_commit_covers_the_whole_store).
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

    /// Writes down the places a declaration claims, at the prefix it claims
    /// them under.
    ///
    /// Every declaration is recorded at the open, from the inventory, because
    /// every one of them has a prefix known before anything is built. This is
    /// the door for a caller with a snapshot in hand - a migration that has
    /// just brought a prefix up to date, a test - rather than the ordinary way
    /// one gets written.
    ///
    /// Without the record the store holds data under a path no recorded schema
    /// claims, which is the one question a tool reading the store on its own
    /// asks. See `RFC-the-ownership-tree.md`.
    ///
    /// Writing the same shape again is not an error and costs nothing: the
    /// engines compare before they write, because a struct is built as often as
    /// the application likes.
    ///
    /// Defaulted, so a backend implemented outside this crate stays correct
    /// without knowing the question exists - at the price of a tool learning
    /// nothing about what such a struct claimed there.
    fn record_schema(&self, at: &StorePath, schema: &SchemaSnapshot) -> StorageResult<()> {
        let _ = (at, schema);
        Ok(())
    }

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
///
/// This is the boundary: a read fails with [`ReadValue`] and a write with
/// [`WriteValue`], each carrying what is known where it was raised. Under it
/// everything still travels as a `Report<StorageError>`, and the `From` impls
/// both ways are what let a `?` cross without a `map_err` at every line.
pub trait StoreExt: StoreBackend {
    fn get<T: DeserializeOwned>(&self, path: impl IntoStorePath) -> ReadResult<Option<T>> {
        let path = to_path(path)?;
        let mut out = None;
        let found = self
            .get_erased(&path, &mut |d| {
                out = Some(
                    erased_serde::deserialize::<T>(d)
                        .map_err(CodecError::from)
                        .change_context(StorageError::Codec)
                        .attach_key(&path)?,
                );
                Ok(())
            })
            .map_err(|why| ReadValue::from_store(&path, why))?;
        Ok(if found { out } else { None })
    }

    fn set<T: Serialize>(&self, path: impl IntoStorePath, value: &T) -> Result<(), WriteValue> {
        let path = to_path(path)?;
        self.set_erased(&path, &value, None)
            .map_err(|why| WriteValue::from_store(&path, why))
    }

    fn set_owned<T: Serialize>(&self, path: StorePath, value: &T) -> Result<(), WriteValue> {
        self.set_owned_erased(path.clone(), &value, None)
            .map_err(|why| WriteValue::from_store(&path, why))
    }

    fn set_with_source<T: Serialize>(
        &self,
        path: impl IntoStorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> Result<(), WriteValue> {
        let path = to_path(path)?;
        self.set_erased(&path, &value, source)
            .map_err(|why| WriteValue::from_store(&path, why))
    }

    fn set_owned_with_source<T: Serialize>(
        &self,
        path: StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> Result<(), WriteValue> {
        self.set_owned_erased(path.clone(), &value, source)
            .map_err(|why| WriteValue::from_store(&path, why))
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

    /// The same, reading through `read` rather than through `T`'s own impl.
    ///
    /// For a field declared `#[amestate(deserialize_with = ..)]`, whose stored
    /// form is not the one its type reads.
    fn decode_with<T>(&self, at: &StorePath, bytes: &[u8], read: ReadAs<T>) -> StorageResult<T> {
        let mut out = None;
        self.decode_erased(bytes, &mut |d| {
            out = Some(
                read(d)
                    .map_err(CodecError::from)
                    .change_context(StorageError::Codec)
                    .attach_with(|| format!("as: {}", std::any::type_name::<T>()))
                    .attach_key(at)?,
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

/// How a field's value is read back, when its own type is not what reads it.
///
/// A `deserialize_with` written the way serde wants one, called with an erased
/// deserializer.
pub type ReadAs<T> =
    for<'de> fn(&mut dyn erased_serde::Deserializer<'de>) -> Result<T, erased_serde::Error>;

/// How it is written.
///
/// The value is wrapped in a `Serialize` that calls its `serialize_with`, and
/// `then` is handed that.
pub type WriteAs<T> =
    fn(&T, &mut dyn FnMut(&dyn erased_serde::Serialize) -> StorageResult<()>) -> StorageResult<()>;

/// How a field is stored, when that is not how its type would be.
pub struct StoredAs<T> {
    pub write: Option<WriteAs<T>>,
    pub read: Option<ReadAs<T>>,
}

impl<T> Default for StoredAs<T> {
    /// Stored the way the type itself would be.
    fn default() -> Self {
        Self {
            write: None,
            read: None,
        }
    }
}

impl<T> Clone for StoredAs<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for StoredAs<T> {}

impl<T> fmt::Debug for StoredAs<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredAs")
            .field("write", &self.write.is_some())
            .field("read", &self.read.is_some())
            .finish()
    }
}

impl<S: StoreBackend + ?Sized> StoreExt for S {}

/// Reactive values addressed by path, without declaring a struct. See [`crate::store::Kv`].
impl Store {
    pub fn kv(&self) -> Kv {
        Kv::new(self.clone())
    }
}
