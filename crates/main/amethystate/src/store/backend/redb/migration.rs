use super::tables::{
    Keyed, TABLE_DATA, TABLE_META, TABLE_MIGRATION_LOG, TABLE_SCHEMA_SNAPSHOT, TableReader,
    TableWriter,
};
use crate::migration::AppliedStep;
use crate::store::CodecFormat;
use crate::store::backend::utils;
use crate::store::error::{StorageError, StorageResult};
use crate::store::facts::Facts;
use crate::store::meta::{PrefixMeta, SchemaSnapshot};
use crate::store::traits::MigrationBackendAdapter;
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use redb::{ReadableTable, TableHandle};
use std::path::Path;

pub(super) struct RedbMigrationBackend<'a> {
    txn: &'a redb::WriteTransaction,
    path: &'a Path,
}

impl<'a> RedbMigrationBackend<'a> {
    pub(super) fn new(txn: &'a redb::WriteTransaction, path: &'a Path) -> Self {
        Self { txn, path }
    }

    fn data_table(&self) -> StorageResult<redb::Table<'_, &'static [u8], &'static [u8]>> {
        self.txn
            .open_table(TABLE_DATA)
            .change_context(StorageError::Migrate)
            .attach_store_file(self.path)
            .attach_table(TABLE_DATA.name())
    }
}

trait Bookkeeping<T> {
    fn bookkeeping(self, store: &Path, table: Keyed, prefix: &StorePath) -> StorageResult<T>;
}

impl<T, E> Bookkeeping<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn bookkeeping(self, store: &Path, table: Keyed, prefix: &StorePath) -> StorageResult<T> {
        self.change_context(StorageError::Meta)
            .attach_store_file(store)
            .attach_table(table.name())
            .attach_prefix(prefix)
    }
}

impl MigrationBackendAdapter for RedbMigrationBackend<'_> {
    fn format(&self) -> CodecFormat {
        CodecFormat::MessagePack
    }

    fn get(&self, key: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        let table = self.data_table()?;
        Ok(table
            .get(key.key().as_bytes())
            .change_context(StorageError::Migrate)
            .attach_store_file(self.path)
            .attach_key(key)?
            .map(|v| v.value().to_vec()))
    }

    fn set(&mut self, key: &StorePath, value: &[u8]) -> StorageResult<()> {
        let mut table = self.data_table()?;
        table
            .insert(key.key().as_bytes(), value)
            .change_context(StorageError::Migrate)
            .attach_store_file(self.path)
            .attach_key(key)
            .attach_value_bytes(value.len())?;
        Ok(())
    }

    fn delete(&mut self, key: &StorePath) -> StorageResult<()> {
        let mut table = self.data_table()?;
        table
            .remove(key.key().as_bytes())
            .change_context(StorageError::Migrate)
            .attach_store_file(self.path)
            .attach_key(key)?;
        Ok(())
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        let under = prefix.key();
        let (low, high) = under.subtree();
        let table = self.data_table()?;
        let mut result = Vec::new();

        let entries = match &high {
            Some(high) => table.range(low..high.as_slice()),
            None => table.range(low..),
        }
        .change_context(StorageError::Migrate)
        .attach_store_file(self.path)
        .attach_prefix(prefix)?;

        for entry in entries {
            let (k, v) = entry
                .change_context(StorageError::Migrate)
                .attach_store_file(self.path)
                .attach_prefix(prefix)
                .attach_read_so_far(result.len())?;
            result.push((utils::stored_path(k.value())?, v.value().to_vec()));
        }
        Ok(result)
    }

    fn get_meta(&self, prefix: &StorePath) -> StorageResult<Option<PrefixMeta>> {
        self.txn
            .load_typed(TABLE_META, utils::prefix_meta_key(prefix).as_bytes())
            .bookkeeping(self.path, TABLE_META, prefix)
    }

    fn set_meta(&mut self, prefix: &StorePath, meta: &PrefixMeta) -> StorageResult<()> {
        self.txn
            .save_typed(TABLE_META, utils::prefix_meta_key(prefix).as_bytes(), meta)
            .bookkeeping(self.path, TABLE_META, prefix)
    }

    fn get_schema_snapshots(&self, prefix: &StorePath) -> StorageResult<Vec<SchemaSnapshot>> {
        Ok(self
            .txn
            .load_typed::<Vec<SchemaSnapshot>>(TABLE_SCHEMA_SNAPSHOT, prefix.key().as_bytes())
            .bookkeeping(self.path, TABLE_SCHEMA_SNAPSHOT, prefix)?
            .unwrap_or_default())
    }

    fn set_schema_snapshots(
        &mut self,
        prefix: &StorePath,
        trees: &[SchemaSnapshot],
    ) -> StorageResult<()> {
        self.txn
            .save_typed(TABLE_SCHEMA_SNAPSHOT, prefix.key().as_bytes(), &trees)
            .bookkeeping(self.path, TABLE_SCHEMA_SNAPSHOT, prefix)
    }

    fn get_migration_log(&self, prefix: &StorePath) -> StorageResult<Option<Vec<AppliedStep>>> {
        self.txn
            .load_typed(TABLE_MIGRATION_LOG, prefix.key().as_bytes())
            .bookkeeping(self.path, TABLE_MIGRATION_LOG, prefix)
    }

    fn set_migration_log(&mut self, prefix: &StorePath, log: &[AppliedStep]) -> StorageResult<()> {
        self.txn
            .save_typed(TABLE_MIGRATION_LOG, prefix.key().as_bytes(), &log)
            .bookkeeping(self.path, TABLE_MIGRATION_LOG, prefix)
            .attach_with(|| format!("steps: {}", log.len()))
    }
}
