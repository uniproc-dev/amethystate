use crate::codec::CodecError;
use crate::observability::InspectorBackend;
use crate::store::CodecFormat;
use crate::store::backend::redb::tables::TABLE_SCHEMA_SNAPSHOT;
use crate::store::backend::utils;
use crate::store::error::StorageError;
use crate::store::facts::Facts;
use crate::store::meta::SchemaSnapshot;
use crate::stores::RedbStore;
use crate::{StorageResult, StoreBackend};
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use redb::{ReadableDatabase, ReadableTable, TableHandle};

impl InspectorBackend for RedbStore {
    fn format(&self) -> CodecFormat {
        CodecFormat::MessagePack
    }

    fn scan_all(&self) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        self.scan_prefix(&StorePath::root())
            .attach("inspecting every key in the store")
    }

    fn get_schema_snapshots(&self) -> StorageResult<Vec<(String, SchemaSnapshot)>> {
        let read_txn = self
            .inner
            .db()?
            .begin_read()
            .change_context(StorageError::Meta)
            .attach_store_file(&self.inner.path)?;
        let table = read_txn
            .open_table(TABLE_SCHEMA_SNAPSHOT)
            .change_context(StorageError::Meta)
            .attach_store_file(&self.inner.path)
            .attach_table(TABLE_SCHEMA_SNAPSHOT.name())?;

        let mut results = Vec::new();
        let entries = table
            .iter()
            .change_context(StorageError::Meta)
            .attach_store_file(&self.inner.path)
            .attach_table(TABLE_SCHEMA_SNAPSHOT.name())?;
        for entry in entries {
            let (k, v) = entry
                .change_context(StorageError::Meta)
                .attach_store_file(&self.inner.path)
                .attach_read_so_far(results.len())?;
            let prefix = k.value().to_string();
            let snapshot: SchemaSnapshot = rmp_serde::from_slice(v.value())
                .map_err(CodecError::from)
                .change_context(StorageError::Meta)
                .attach_store_file(&self.inner.path)
                .attach_raw_key(&prefix)
                .attach_value_bytes(v.value().len())?;
            results.push((prefix, snapshot));
        }
        Ok(results)
    }

    fn set_raw(&mut self, key: &str, value: &[u8]) -> StorageResult<()> {
        self.inner.check_debouncer()?;
        let path = utils::stored_path(key)?;
        utils::set_raw_pending(
            &self.inner.pending,
            &self.inner.subscriptions,
            &self.inner.debouncer,
            &path,
            value,
        )
        .attach_store_file(&self.inner.path)
        .attach_key(&path)
    }
}
