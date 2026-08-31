use super::toml_doc::TomlDocument;
use crate::migration::set::MigrationSet;
use crate::store::backend::text::store::TextStore;
use crate::store::config::StoreConfig;
use crate::store::{StoreBackend, StoreCallback, SubscriptionId, SubscriptionKind};
use crate::{MigrationReport, StorageResult};
use amethystate_core::path::StorePath;
use uuid::Uuid;

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct TomlStore(pub TextStore<TomlDocument>);

impl TomlStore {
    pub fn open(
        config: StoreConfig,
        migration_set: MigrationSet,
    ) -> StorageResult<(Self, MigrationReport)> {
        let (store, report) = TextStore::open(config, migration_set)?;
        Ok((TomlStore(store), report))
    }
}

impl StoreBackend for TomlStore {
    fn get_raw(&self, path: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        self.0.get_raw(path)
    }

    fn get_erased(
        &self,
        path: &StorePath,
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<bool> {
        self.0.get_erased(path, f)
    }

    fn decode_erased(
        &self,
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()> {
        self.0.decode_erased(bytes, f)
    }

    fn set_erased(
        &self,
        path: &StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.0.set_erased(path, value, source)
    }

    fn set_owned_erased(
        &self,
        path: StorePath,
        value: &dyn erased_serde::Serialize,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.0.set_owned_erased(path, value, source)
    }

    fn delete_with_source(&self, path: &StorePath, source: Option<Uuid>) -> StorageResult<()> {
        self.0.delete_with_source(path, source)
    }

    fn delete_prefix_with_source(
        &self,
        prefix: &StorePath,
        source: Option<Uuid>,
    ) -> StorageResult<()> {
        self.0.delete_prefix_with_source(prefix, source)
    }

    fn delete(&self, path: &StorePath) -> StorageResult<()> {
        self.0.delete(path)
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        self.0.scan_prefix(prefix)
    }

    fn files(&self) -> Option<crate::store::traits::StoreLayout> {
        self.0.files()
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        self.0.scan_keys(prefix)
    }

    fn save_now(&self) -> StorageResult<()> {
        self.0.save_now()
    }

    fn close(&self) -> StorageResult<()> {
        self.0.close()
    }

    fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    fn subscribe(&self, kind: SubscriptionKind, callback: StoreCallback) -> SubscriptionId {
        self.0.subscribe(kind, callback)
    }

    fn unsubscribe(&self, id: SubscriptionId) {
        self.0.unsubscribe(id)
    }

    fn flush_prefix(&self, prefix: &StorePath) -> StorageResult<()> {
        self.0.flush_prefix(prefix)
    }
    fn flush_async(&self) -> crate::store::durable::Commit {
        self.0.flush_async()
    }

    fn is_initialized(&self, namespace: &StorePath) -> StorageResult<bool> {
        self.0.is_initialized(namespace)
    }

    fn set_initialized(
        &self,
        namespace: &StorePath,
        state: crate::store::InitState,
    ) -> StorageResult<()> {
        self.0.set_initialized(namespace, state)
    }
}

crate::define_store_test_suite!(
    TomlStore,
    "toml",
    "[amethystate]\nwatch_interval_ms = 50\n\n[ui.theme]\ndark = false",
    "[amethystate]\nwatch_interval_ms = 50\n\n[ui.theme]\ndark = true",
    "[amethystate]\nwatch_interval_ms = 50\n\n[ui]\ntheme = {}"
);
