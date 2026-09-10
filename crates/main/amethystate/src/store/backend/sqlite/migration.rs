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

    fn get_typed<T: DeserializeOwned>(
        &self,
        table: &str,
        at: &StorePath,
    ) -> StorageResult<Option<T>> {
        let sql = format!("SELECT value FROM {} WHERE key = ?", table);
        let mut stmt = self
            .txn
            .prepare_cached(&sql)
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Meta)
            .attach_table(table)
            .attach_key(at)?;
        let res: Option<Vec<u8>> = stmt
            .query_row([at.key().as_bytes()], |row| row.get(0))
            .optional()
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Meta)
            .attach_table(table)
            .attach_key(at)?;

        match res {
            Some(bytes) => Ok(Some(
                sonic_rs::from_slice(&bytes)
                    .map_err(CodecError::from)
                    .change_context(StorageError::Codec)
                    .attach_table(table)
                    .attach_key(at)
                    .attach_value_bytes(bytes.len())?,
            )),
            None => Ok(None),
        }
    }

    fn set_typed<T: Serialize>(
        &self,
        table: &str,
        at: &StorePath,
        value: &T,
    ) -> StorageResult<()> {
        let bytes = sonic_rs::to_vec(value)
            .map_err(CodecError::from)
            .change_context(StorageError::Codec)
            .attach_table(table)
            .attach_key(at)?;

        let sql = format!("REPLACE INTO {} (key, value) VALUES (?, ?)", table);
        let mut stmt = self
            .txn
            .prepare_cached(&sql)
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Meta)
            .attach_table(table)
            .attach_key(at)?;
        stmt.execute(rusqlite::params![at.key().as_bytes(), bytes])
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Meta)
            .attach_table(table)
            .attach_key(at)?;
        Ok(())
    }
}


impl MigrationBackendAdapter for SqliteMigrationBackend<'_> {
    fn format(&self) -> CodecFormat {
        CodecFormat::SonicJson
    }

    fn get(&self, key: &StorePath) -> StorageResult<Option<Vec<u8>>> {
        let mut stmt = self
            .txn
            .prepare_cached("SELECT value FROM data WHERE key = ?")
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Read)
            .attach_key(key)?;
        stmt.query_row([key.key().as_bytes()], |row| row.get(0))
            .optional()
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Read)
            .attach_key(key)
    }

    fn set(&mut self, key: &StorePath, value: &[u8]) -> StorageResult<()> {
        let mut stmt = self
            .txn
            .prepare_cached("REPLACE INTO data (key, value) VALUES (?, ?)")
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Write)
            .attach_key(key)?;
        stmt.execute(rusqlite::params![key.key().as_bytes(), value])
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Write)
            .attach_key(key)
            .attach_value_bytes(value.len())?;
        Ok(())
    }

    fn delete(&mut self, key: &StorePath) -> StorageResult<()> {
        let mut stmt = self
            .txn
            .prepare_cached("DELETE FROM data WHERE key = ?")
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Delete)
            .attach_key(key)?;
        stmt.execute([key.key().as_bytes()])
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Delete)
            .attach_key(key)?;
        Ok(())
    }

    /// The rows under `prefix`, by comparison rather than by pattern.
    ///
    /// A pattern would be two mistakes at once. `GLOB` is a pattern language,
    /// so a name holding `*`, `?` or `[` - all legal in a name, and
    /// `a_key_carries_what_a_glob_pattern_reads` in `path.rs` says so - reads
    /// as a pattern rather than as itself. And `prefix*` is not "under": two
    /// map fields named `routes` and `routes_v2` share a beginning, so loading
    /// the first picks up the second's entries and then refuses them for not
    /// being under the map they were scanned from, failing the migration for
    /// good.
    ///
    /// The store's own scans ask the subtree; this is the same thing, in the
    /// adapter that repairs data rather than serves it.
    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        let under = prefix.key();
        let (low, high) = under.subtree();

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
            .query_map(rusqlite::params![low, high.as_deref()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(SqliteStoreError::from)
            .change_context(StorageError::Scan)
            .attach_prefix(prefix)?;

        let mut res = Vec::new();
        for row in rows {
            let (key, value): (Vec<u8>, Vec<u8>) = row
                .map_err(SqliteStoreError::from)
                .change_context(StorageError::Scan)
                .attach_prefix(prefix)
                .attach_read_so_far(res.len())?;

            res.push((crate::store::backend::utils::stored_path(&key)?, value));
        }
        Ok(res)
    }

    fn get_meta(&self, prefix: &StorePath) -> StorageResult<Option<PrefixMeta>> {
        self.get_typed("metadata", prefix)
    }
    fn set_meta(&mut self, prefix: &StorePath, meta: &PrefixMeta) -> StorageResult<()> {
        self.set_typed("metadata", prefix, meta)
    }

    fn get_schema_snapshots(&self, prefix: &StorePath) -> StorageResult<Vec<SchemaSnapshot>> {
        Ok(self
            .get_typed::<Vec<SchemaSnapshot>>("schema_snapshot", prefix)?
            .unwrap_or_default())
    }
    fn set_schema_snapshots(
        &mut self,
        prefix: &StorePath,
        trees: &[SchemaSnapshot],
    ) -> StorageResult<()> {
        self.set_typed("schema_snapshot", prefix, &trees)
    }

    fn get_migration_log(&self, prefix: &StorePath) -> StorageResult<Option<Vec<AppliedStep>>> {
        self.get_typed("migration_log", prefix)
    }
    fn set_migration_log(&mut self, prefix: &StorePath, log: &[AppliedStep]) -> StorageResult<()> {
        self.set_typed("migration_log", prefix, &log)
    }
}
