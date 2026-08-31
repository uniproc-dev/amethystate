use crate::store::backend;

#[cfg(feature = "json")]
pub use backend::text::JsonStore;

#[cfg(feature = "sqlite")]
pub use backend::sqlite::SqliteStore;

#[cfg(feature = "redb")]
pub use backend::redb::RedbStore;

#[cfg(feature = "toml")]
pub use backend::text::TomlStore;

#[cfg(feature = "ron")]
pub use backend::text::RonStore;

use crate::MigrationReport;
use crate::store::config::StoreConfig;
use crate::store::{
    InitState, StorageResult, StoreBackend, StoreCallback, StoreExt, SubscriptionId, to_path,
};
use amethystate_core::path::{IntoStorePath, StorePath};
use std::sync::Arc;

/// A handle on an open store.
///
/// One type over every engine - which one is behind it is settled by
/// [`StoreBuilder`](crate::StoreBuilder) when the store is opened, and nothing
/// downstream is generic over it.
///
/// Cheap to clone and shared by every clone: the file stays open as long as one
/// handle is alive, and closes when the last is dropped.
///
/// Dropping the last handle writes what is still buffered, but it is the one
/// write whose failure it cannot report: `Drop` has no caller to return an
/// error to, and by then there is rarely anyone left to tell. An application
/// that would rather find out while it can still act - offer to retry, save
/// somewhere else, or not exit yet - calls [`Store::close`] on the way out and
/// reads the result.
#[derive(Clone)]
pub struct Store {
    backend: Arc<dyn StoreBackend>,
    owners: Arc<crate::store::owners::Owners>,
    context: Arc<crate::store::CheckContext>,
}

impl Store {
    /// Wraps a backend that was built by hand.
    ///
    /// [`StoreBuilder`](crate::StoreBuilder) is the ordinary way in; this is
    /// for a [`StoreBackend`] implemented outside the crate.
    pub fn from_arc(inner: Arc<dyn StoreBackend>) -> Self {
        Self {
            backend: inner,
            owners: Arc::new(crate::store::owners::Owners::default()),
            context: Arc::new(crate::store::CheckContext::default()),
        }
    }

    /// What the application handed this store for its declared checks, put
    /// there by [`StoreBuilder::context`](crate::StoreBuilder::context).
    ///
    /// A store opened any other way carries an empty one, and a check asking
    /// it for something is refused with what was on offer.
    pub fn context(&self) -> &crate::store::CheckContext {
        &self.context
    }

    pub(crate) fn with_context(mut self, context: Arc<crate::store::CheckContext>) -> Self {
        self.context = context;
        self
    }

    /// The erased backend underneath, for code that is generic over
    /// [`StoreBackend`] rather than over this handle.
    pub fn as_dyn(&self) -> &Arc<dyn StoreBackend> {
        &self.backend
    }

    /// Who owns which stored path, shared by every clone of this handle.
    pub fn owners(&self) -> &crate::store::owners::Owners {
        &self.owners
    }

    /// Opens the store with [`crate::store::builder::default_backend`].
    pub fn open(
        config: StoreConfig,
        mset: crate::migration::set::MigrationSet,
    ) -> StorageResult<(Self, MigrationReport)> {
        crate::store::builder::default_backend().open_public(config, mset)
    }
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Store")
    }
}

impl PartialEq for Store {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.backend, &other.backend)
    }
}
impl Eq for Store {}

impl std::ops::Deref for Store {
    type Target = dyn StoreBackend;
    fn deref(&self) -> &Self::Target {
        &*self.backend
    }
}

/// The typed surface, inherent so a call site needs no trait in scope.
impl Store {
    /// Reads a value by path, or `None` if nothing is stored there.
    ///
    /// Sees buffered writes as well as committed ones. The type is whatever
    /// you ask for and nothing remembers it - [`Kv::cell`](crate::store::Kv::cell)
    /// is the variant that does.
    ///
    /// The levels are given as a list, and a string is not one of them - a name
    /// holding a separator is a name, and nesting is spelled out:
    ///
    /// ```
    /// # use amethystate::StoreBuilder;
    /// # let path = amethystate_core::test_utils::TempPath::new("doc");
    /// # let store = StoreBuilder::new(&*path).build().unwrap();
    /// store.set(["ui", "width"], &1280u32).unwrap();
    ///
    /// assert_eq!(store.get::<u32>(["ui", "width"]).unwrap(), Some(1280));
    /// assert_eq!(store.get::<u32>(["ui", "height"]).unwrap(), None);
    ///
    /// // One level called `ui.width`, which is somewhere else entirely.
    /// store.set(["ui.width"], &7u32).unwrap();
    /// assert_eq!(store.get::<u32>(["ui", "width"]).unwrap(), Some(1280));
    /// ```
    pub fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: impl IntoStorePath,
    ) -> StorageResult<Option<T>> {
        StoreExt::get(self, path)
    }
    /// Writes a value at `path`, creating it or replacing what was there.
    ///
    /// The write lands in the buffer and notifies subscribers; the debouncer
    /// commits it later. Nothing here carries provenance, so a subscription
    /// cannot tell this apart from its own write - use
    /// [`Store::set_with_source`] when it must.
    pub fn set<T: serde::Serialize>(
        &self,
        path: impl IntoStorePath,
        value: &T,
    ) -> StorageResult<()> {
        StoreExt::set(self, path, value)
    }
    /// [`Store::set`] for a path already owned, saving the clone the borrowed
    /// form would make.
    pub fn set_owned<T: serde::Serialize>(&self, path: StorePath, value: &T) -> StorageResult<()> {
        StoreExt::set_owned(self, path, value)
    }
    /// [`Store::set`] tagged with who made the write.
    ///
    /// Subscribers receive the id, which is how a component ignores the echo
    /// of its own change instead of reacting to it.
    pub fn set_with_source<T: serde::Serialize>(
        &self,
        path: impl IntoStorePath,
        value: &T,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        StoreExt::set_with_source(self, path, value, source)
    }
    /// [`Store::set_with_source`] for a path already owned.
    pub fn set_owned_with_source<T: serde::Serialize>(
        &self,
        path: StorePath,
        value: &T,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        StoreExt::set_owned_with_source(self, path, value, source)
    }
    /// Removes whatever is at `path`. Removing an absent path is not an error.
    pub fn delete(&self, path: impl IntoStorePath) -> StorageResult<()> {
        StoreBackend::delete(self, &to_path(path)?)
    }

    /// Removes every level under `prefix`, and the value at `prefix` itself.
    pub fn delete_prefix(&self, prefix: impl IntoStorePath) -> StorageResult<()> {
        StoreBackend::delete_prefix(self, &to_path(prefix)?)
    }

    /// Commits what is buffered under `prefix`.
    pub fn flush_prefix(&self, prefix: impl IntoStorePath) -> StorageResult<()> {
        StoreBackend::flush_prefix(self, &to_path(prefix)?)
    }

    /// The keys under `prefix`, sorted, without reading their values.
    pub fn scan_keys(&self, prefix: impl IntoStorePath) -> StorageResult<Vec<StorePath>> {
        StoreBackend::scan_keys(self, &to_path(prefix)?)
    }

    /// Every key under `prefix` with its bytes, sorted by key.
    pub fn scan_prefix(
        &self,
        prefix: impl IntoStorePath,
    ) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        StoreBackend::scan_prefix(self, &to_path(prefix)?)
    }

    /// Decodes bytes that arrived in a [`StoreEvent`](crate::StoreEvent),
    /// in whatever format this backend writes.
    ///
    /// Bytes that will not decode - because the file was edited by hand, or the
    /// field changed type - are an error naming the type that was asked for and
    /// what the codec made of them, so a caller can tell a stored default from
    /// a value it could not read.
    pub fn decode<T: serde::de::DeserializeOwned>(&self, bytes: &[u8]) -> StorageResult<T> {
        StoreExt::decode(self, bytes)
    }
}

/// Addressing the store by path, with nothing in the way - and what that costs
/// is written on [`StoreBackend`] itself, which is worth reading before
/// reaching for any of it. The short of it: a write here lands wherever it is
/// aimed, declared paths included, and
/// [`Kv`](crate::store::Kv) is the surface that refuses to.
///
/// [`Store::get`] and [`Store::set`] are the same operations under a shorter
/// name, so they carry the same warning.
impl StoreBackend for Store {
    fn get_raw(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        self.backend.get_raw(path)
    }
    fn set_erased(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.backend.set_erased(path, value, source)
    }
    fn set_owned_erased(
        &self,
        path: StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.backend.set_owned_erased(path, value, source)
    }
    fn get_erased(
        &self,
        path: &StorePath,
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<bool> {
        self.backend.get_erased(path, f)
    }
    fn decode_erased(
        &self,
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()> {
        self.backend.decode_erased(bytes, f)
    }
    fn delete_with_source(
        &self,
        path: &StorePath,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.backend.delete_with_source(path, source)
    }
    fn delete(&self, path: &StorePath) -> StorageResult<()> {
        self.backend.delete(path)
    }
    fn delete_prefix_with_source(
        &self,
        prefix: &StorePath,
        source: Option<uuid::Uuid>,
    ) -> StorageResult<()> {
        self.backend.delete_prefix_with_source(prefix, source)
    }
    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        self.backend.scan_prefix(prefix)
    }
    fn visit_prefix(
        &self,
        prefix: &StorePath,
        visit: &mut dyn FnMut(&str, &[u8]) -> StorageResult<()>,
    ) -> StorageResult<()> {
        self.backend.visit_prefix(prefix, visit)
    }
    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        self.backend.scan_keys(prefix)
    }
    fn parallel_reads(&self) -> bool {
        self.backend.parallel_reads()
    }
    fn files(&self) -> Option<crate::store::traits::StoreLayout> {
        self.backend.files()
    }
    fn save_now(&self) -> StorageResult<()> {
        self.backend.save_now()
    }
    fn close(&self) -> StorageResult<()> {
        self.backend.close()
    }
    fn is_closed(&self) -> bool {
        self.backend.is_closed()
    }
    fn subscribe(&self, kind: crate::SubscriptionKind, callback: StoreCallback) -> SubscriptionId {
        self.backend.subscribe(kind, callback)
    }
    fn unsubscribe(&self, id: SubscriptionId) {
        self.backend.unsubscribe(id)
    }
    fn flush_prefix(&self, prefix: &StorePath) -> StorageResult<()> {
        self.backend.flush_prefix(prefix)
    }
    fn flush_async(&self) -> crate::store::durable::Commit {
        self.backend.flush_async()
    }
    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool> {
        self.backend.is_initialized(namespace)
    }
    fn set_initialized(&self, namespace: &StorePath, state: InitState) -> StorageResult<()> {
        self.backend.set_initialized(namespace, state)
    }
}
