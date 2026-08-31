use crate::Store;
use crate::StoreBackend;
use amethystate_core::path::StorePath;

use crate::store::StorageError;
use amethystate_core::AmeBackendSync;
use error_stack::Report;
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

pub(crate) struct SyncBridge {
    pub(crate) store: Store,
}

impl SyncBridge {
    pub(crate) fn new(store: Store) -> Self {
        Self { store }
    }
}

impl AmeBackendSync for SyncBridge {
    type Error = StorageError;
    type Raw = Vec<u8>;
    type Borrowed = [u8];

    fn get<T>(&self, path: &StorePath) -> Result<Option<T>, Report<Self::Error>>
    where
        T: DeserializeOwned,
    {
        self.store.get(path)
    }

    fn set_with_source<T: Serialize>(
        &self,
        path: &StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>> {
        self.store.set_with_source(path, value, source)
    }

    fn set_owned_with_source<T: Serialize>(
        &self,
        path: StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>> {
        self.store.set_owned_with_source(path, value, source)
    }

    fn set<T>(&self, path: &StorePath, value: &T) -> Result<(), Report<Self::Error>>
    where
        T: Serialize,
    {
        self.store.set(path, value)
    }

    fn delete(&self, path: &StorePath) -> Result<(), Report<Self::Error>> {
        self.store.delete(path)
    }

    fn delete_with_source(
        &self,
        path: &StorePath,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>> {
        self.store.delete_with_source(path, source)
    }

    fn delete_prefix(
        &self,
        prefix: &StorePath,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>> {
        self.store.delete_prefix_with_source(prefix, source)
    }

    fn scan_prefix(
        &self,
        prefix: &StorePath,
    ) -> Result<Vec<(StorePath, Self::Raw)>, Report<Self::Error>> {
        self.store.scan_prefix(prefix)
    }

    fn scan_keys(&self, prefix: &StorePath) -> Result<Vec<StorePath>, Report<Self::Error>> {
        self.store.scan_keys(prefix)
    }

    fn decode<T>(&self, raw: &[u8]) -> Result<T, Report<Self::Error>>
    where
        T: DeserializeOwned + Default,
    {
        self.store.decode(raw)
    }
}
