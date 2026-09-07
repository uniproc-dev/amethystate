use crate::failure::StorageResult;
use crate::path::StorePath;
#[cfg(feature = "async")]
use error_stack::Report;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::borrow::Borrow;
use uuid::Uuid;

/// A store that answers on the calling thread.
///
/// It fails with [`StorageError`](crate::failure::StorageError) rather than an error of its own: what an
/// engine said is the frame below, and telling a depth refusal from a codec
/// one at the point a caller has to act on it needs the kind to survive the
/// trip. An associated type would put it behind a bound nobody could name.
pub trait AmeBackendSync {
    type Raw: Borrow<Self::Borrowed>;
    type Borrowed: ?Sized;

    fn get<T>(&self, path: &StorePath) -> StorageResult<Option<T>>
    where
        T: DeserializeOwned;

    fn set_owned<T: Serialize>(&self, path: StorePath, value: &T) -> StorageResult<()> {
        self.set(&path, value)
    }

    fn set_with_source<T: Serialize>(
        &self,
        path: &StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> StorageResult<()>;
    fn set_owned_with_source<T: Serialize>(
        &self,
        path: StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> StorageResult<()>;

    fn set<T>(&self, path: &StorePath, value: &T) -> StorageResult<()>
    where
        T: Serialize;

    fn delete(&self, path: &StorePath) -> StorageResult<()>;

    fn delete_with_source(&self, path: &StorePath, source: Option<Uuid>) -> StorageResult<()>;

    /// Removes every key under `prefix` as one operation, emitting a single
    /// event rather than one per key.
    fn delete_prefix(&self, prefix: &StorePath, source: Option<Uuid>) -> StorageResult<()>;

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Self::Raw)>>;

    /// The keys under `prefix`, sorted, without reading their values.
    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>>;

    fn decode<T>(&self, raw: &Self::Borrowed) -> StorageResult<T>
    where
        T: DeserializeOwned + Default;
}
#[cfg(feature = "async")]
#[allow(async_fn_in_trait)]
pub trait AmeBackendAsync {
    type Error: std::error::Error + Send + Sync + 'static;
    type Raw;

    async fn get<T>(&self, path: &StorePath) -> Result<Option<T>, Report<Self::Error>>
    where
        T: DeserializeOwned;

    async fn set<T>(&self, path: &StorePath, value: &T) -> Result<(), Report<Self::Error>>
    where
        T: Serialize;
    async fn set_with_source<T: Serialize>(
        &self,
        path: &StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>>;
    async fn set_owned_with_source<T: Serialize>(
        &self,
        path: StorePath,
        value: &T,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>>;

    async fn delete(&self, path: &StorePath) -> Result<(), Report<Self::Error>>;
    async fn delete_with_source(
        &self,
        path: &StorePath,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>>;

    /// Removes the whole subtree as one operation, so subscribers hear one
    /// change at the prefix rather than one per key.
    async fn delete_prefix(
        &self,
        prefix: &StorePath,
        source: Option<Uuid>,
    ) -> Result<(), Report<Self::Error>>;

    async fn scan_prefix(
        &self,
        prefix: &StorePath,
    ) -> Result<Vec<(StorePath, Self::Raw)>, Report<Self::Error>>;

    async fn scan_keys(&self, prefix: &StorePath) -> Result<Vec<StorePath>, Report<Self::Error>>;

    fn decode<T>(&self, raw: &Self::Raw) -> Result<T, Report<Self::Error>>
    where
        T: DeserializeOwned + Default;
}
