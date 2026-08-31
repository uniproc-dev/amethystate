use super::error::SqliteStoreError;
use crate::codec::CodecError;
use crate::migration::AppliedStep;
use crate::store::error::StorageError;
use crate::store::facts::Facts;
use crate::store::meta::{PrefixMeta, SchemaSnapshot};
use crate::store::traits::MigrationBackendAdapter;
use crate::store::{CodecFormat, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::ResultExt;
use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub struct SqliteMigrationBackend<'a> {
    pub(crate) txn: &'a Transaction<'a>,
}

impl<'a> SqliteMigrationBackend<'a> {
    pub fn new(txn: &'a Transaction<'a>) -> Self {
        Self { txn }
    }

    fn get_typed<T: DeserializeOwned>(&self, table: &str, key: &str) -> StorageResult<Option<T>> {
        let sql = format!("SELECT value FROM {} WHERE key = ?", table);
        let mut stmt = self
            .txn
            .prepare_cached(&sql)
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Meta)
            .attach_table(table)
            .attach_raw_key(key)?;
        let res: Option<Vec<u8>> = stmt
            .query_row([key], |row| row.get(0))
            .optional()
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Meta)
            .attach_table(table)
            .attach_raw_key(key)?;

        match res {
            Some(bytes) => Ok(Some(
                sonic_rs::from_slice(&bytes)
                    .map_err(CodecError::from)
                    .change_context(StorageError::Codec)
                    .attach_table(table)
                    .attach_raw_key(key)
                    .attach_value_bytes(bytes.len())?,
            )),
            None => Ok(None),
        }
    }

    fn set_typed<T: Serialize>(&self, table: &str, key: &str, value: &T) -> StorageResult<()> {
        let bytes = sonic_rs::to_vec(value)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec)
            .attach_table(table)
            .attach_raw_key(key)?;

        let sql = format!("REPLACE INTO {} (key, value) VALUES (?, ?)", table);
        let mut stmt = self
            .txn
            .prepare_cached(&sql)
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Meta)
            .attach_table(table)
            .attach_raw_key(key)?;
        stmt.execute(rusqlite::params![key, bytes])
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Meta)
            .attach_table(table)
            .attach_raw_key(key)?;
        Ok(())
    }
}

impl MigrationBackendAdapter for SqliteMigrationBackend<'_> {
    fn format(&self) -> CodecFormat {
        CodecFormat::SonicJson
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let mut stmt = self
            .txn
            .prepare_cached("SELECT value FROM data WHERE key = ?")
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Read)
            .attach_raw_key(key)?;
        stmt.query_row([key], |row| row.get(0))
            .optional()
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Read)
            .attach_raw_key(key)
    }

    fn set(&mut self, key: &str, value: &[u8]) -> StorageResult<()> {
        let mut stmt = self
            .txn
            .prepare_cached("REPLACE INTO data (key, value) VALUES (?, ?)")
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Write)
            .attach_raw_key(key)?;
        stmt.execute(rusqlite::params![key, value])
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Write)
            .attach_raw_key(key)
            .attach_value_bytes(value.len())?;
        Ok(())
    }

    fn delete(&mut self, key: &str) -> StorageResult<()> {
        let mut stmt = self
            .txn
            .prepare_cached("DELETE FROM data WHERE key = ?")
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Delete)
            .attach_raw_key(key)?;
        stmt.execute([key])
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Delete)
            .attach_raw_key(key)?;
        Ok(())
    }

    /// The rows under `prefix`, by comparison rather than by pattern.
    ///
    /// `GLOB '{prefix}*'` was two mistakes at once. It is a pattern language, so
    /// a name holding `*`, `?` or `[` - all legal in a name, and
    /// `a_key_carries_what_a_glob_pattern_reads` in `path.rs` says so - is read
    /// as a pattern rather than as itself. And `prefix*` is not "under": two map
    /// fields named `routes` and `routes_v2` share a beginning, so loading the
    /// first picked up the second's entries and then refused them for not being
    /// under the map they were scanned from, failing the migration for good.
    ///
    /// The store's own scans have asked the subtree all along; this is the same
    /// thing, in the adapter that repairs data rather than serves it.
    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        let subtree = prefix.subtree();
        let (low, high) = subtree.range();

        let mut stmt = self
            .txn
            .prepare_cached(
                "SELECT key, value FROM data \
                 WHERE key >= ?1 AND (?2 IS NULL OR key < ?2) ORDER BY key",
            )
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Scan)
            .attach_prefix(prefix)?;
        let rows = stmt
            .query_map(rusqlite::params![&low, &high], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Scan)
            .attach_prefix(prefix)?;

        let mut res = Vec::new();
        for row in rows {
            let (key, value): (String, Vec<u8>) = row
                .map_err(SqliteStoreError::from)
                .change_context(StorageError::Scan)
                .attach_prefix(prefix)
                .attach_read_so_far(res.len())?;

            if subtree.contains(&key) {
                res.push((crate::store::backend::utils::stored_path(&key)?, value));
            }
        }
        Ok(res)
    }

    fn get_meta(&self, prefix: &StorePath) -> StorageResult<Option<PrefixMeta>> {
        self.get_typed("metadata", prefix.as_str())
    }
    fn set_meta(&mut self, prefix: &StorePath, meta: &PrefixMeta) -> StorageResult<()> {
        self.set_typed("metadata", prefix.as_str(), meta)
    }

    fn get_schema_snapshot(&self, prefix: &StorePath) -> StorageResult<Option<SchemaSnapshot>> {
        self.get_typed("schema_snapshot", prefix.as_str())
    }
    fn set_schema_snapshot(
        &mut self,
        prefix: &StorePath,
        snapshot: &SchemaSnapshot,
    ) -> StorageResult<()> {
        self.set_typed("schema_snapshot", prefix.as_str(), snapshot)
    }

    fn get_migration_log(&self, prefix: &StorePath) -> StorageResult<Option<Vec<AppliedStep>>> {
        self.get_typed("migration_log", prefix.as_str())
    }
    fn set_migration_log(&mut self, prefix: &StorePath, log: &[AppliedStep]) -> StorageResult<()> {
        self.set_typed("migration_log", prefix.as_str(), &log)
    }
}
