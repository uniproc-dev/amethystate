use super::error::RedbResult;
use crate::codec::CodecError;
use redb::{ReadTransaction, TableDefinition, WriteTransaction};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Every table is keyed by bytes, which is what a path encodes to.
///
/// Byte order over those keys is the order of the level lists they came from,
/// so a subtree is a byte prefix and redb's own range is exact - see
/// [`Key`](amethystate_core::path::Key).
pub(super) type Keyed = TableDefinition<'static, &'static [u8], &'static [u8]>;

pub(super) const TABLE_DATA: Keyed = TableDefinition::new("data");
pub(super) const TABLE_META: Keyed = TableDefinition::new("metadata");
pub(super) const TABLE_DIFF_LOG: Keyed = TableDefinition::new("diff_log");
pub(super) const TABLE_MIGRATION_LOG: Keyed = TableDefinition::new("migration_log");
pub(super) const TABLE_SCHEMA_SNAPSHOT: Keyed = TableDefinition::new("schema_snapshot");

pub(super) trait TableReader {
    fn load_typed<T: DeserializeOwned>(
        &self,
        table_def: Keyed,
        key: &[u8],
    ) -> RedbResult<Option<T>>;
}

pub(super) trait TableWriter {
    fn save_typed<T: Serialize>(
        &self,
        table_def: Keyed,
        key: &[u8],
        val: &T,
    ) -> RedbResult<()>;
}

fn deserialize_from_table<T: DeserializeOwned>(
    table: impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    key: &[u8],
) -> RedbResult<Option<T>> {
    table
        .get(key)?
        .map(|v| rmp_serde::from_slice(v.value()).map_err(CodecError::from))
        .transpose()
        .map_err(Into::into)
}

impl TableReader for ReadTransaction {
    fn load_typed<T: DeserializeOwned>(
        &self,
        table_def: Keyed,
        key: &[u8],
    ) -> RedbResult<Option<T>> {
        let table = self.open_table(table_def)?;
        deserialize_from_table(table, key)
    }
}

impl TableReader for WriteTransaction {
    fn load_typed<T: DeserializeOwned>(
        &self,
        table_def: Keyed,
        key: &[u8],
    ) -> RedbResult<Option<T>> {
        let table = self.open_table(table_def)?;
        deserialize_from_table(table, key)
    }
}

impl TableWriter for WriteTransaction {
    fn save_typed<T: Serialize>(
        &self,
        table_def: Keyed,
        key: &[u8],
        val: &T,
    ) -> RedbResult<()> {
        let mut table = self.open_table(table_def)?;
        let bytes = rmp_serde::to_vec_named(val).map_err(CodecError::from)?;
        table.insert(key, bytes.as_slice())?;
        Ok(())
    }
}
