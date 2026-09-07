use crate::migration::AppliedStep;
use crate::store::CodecFormat;
use crate::store::StorageError;
use crate::store::StorageResult;
use crate::store::backend::text::document::TextDocument;
use crate::store::backend::text::layout;
use crate::store::backend::text::store;
use crate::store::declared::Declared;
use crate::store::facts::Facts;
use crate::store::meta::{PrefixMeta, SchemaSnapshot};
use crate::store::screening::Noticed;
use crate::store::traits::MigrationBackendAdapter;
use amethystate_core::path::StorePath;
use error_stack::ResultExt;

fn migration_path(key: &str) -> StorageResult<StorePath> {
    StorePath::parse_joined(key)
        .change_context(StorageError::Path)
        .attach_raw_key(key)
}

pub struct TextMigrationBackend<'a, D: TextDocument> {
    pub(crate) data_doc: &'a mut D,
    pub(crate) meta_doc: &'a mut D,

    /// Judged when the files were read, before anything this open writes into
    /// the metadata put keys back in it. See
    /// [`MigrationBackendAdapter::bookkeeping_is_lost`].
    pub(crate) bookkeeping_is_lost: bool,
}

impl<D: TextDocument> TextMigrationBackend<'_, D> {
    /// Read fresh each time, because a migration is what writes the schemas
    /// this reads - and a step that has just recorded one addresses the file by
    /// it on the next call.
    fn declared(&self) -> StorageResult<Declared> {
        store::declared_in(self.meta_doc)
            .change_context(StorageError::Migrate)
            .attach("reading the recorded schemas the file is laid out by")
    }
}

impl<D: TextDocument> MigrationBackendAdapter for TextMigrationBackend<'_, D> {
    fn format(&self) -> CodecFormat {
        D::format()
    }

    fn bookkeeping_is_lost(&self) -> bool {
        self.bookkeeping_is_lost
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let path = migration_path(key)?;
        let at = layout::levels(self.data_doc, &self.declared()?, &path);
        if let Some(node) = self.data_doc.get(&at) {
            Ok(Some(
                D::node_to_bytes(node)
                    .change_context(StorageError::Migrate)
                    .attach_key(&path)
                    .attach("reading the data file through the migration adapter")?,
            ))
        } else {
            Ok(None)
        }
    }

    fn set(&mut self, key: &str, value: &[u8]) -> StorageResult<()> {
        let path = migration_path(key)?;
        let at = layout::levels(self.data_doc, &self.declared()?, &path);
        let node = D::bytes_to_node(value)
            .change_context(StorageError::Migrate)
            .attach_key(&path)
            .attach_value_bytes(value.len())
            .attach("writing the data file through the migration adapter")?;
        self.data_doc
            .set(&at, node)
            .change_context(StorageError::Migrate)
            .attach_key(&path)
            .attach("writing the data file through the migration adapter")?;
        Ok(())
    }

    fn delete(&mut self, key: &str) -> StorageResult<()> {
        let path = migration_path(key)?;
        let at = layout::levels(self.data_doc, &self.declared()?, &path);
        self.data_doc
            .delete(&at)
            .change_context(StorageError::Migrate)
            .attach_key(&path)
            .attach("deleting from the data file through the migration adapter")?;
        Ok(())
    }

    fn scan_prefix(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        store::scan_prefix_impl(self.data_doc, prefix, &self.declared()?)
            .change_context(StorageError::Migrate)
            .attach_prefix(prefix)
            .attach("scanning the data file through the migration adapter")
    }

    fn get_meta(&self, prefix: &StorePath) -> StorageResult<Option<PrefixMeta>> {
        let key = store::meta_key("meta", prefix);
        if let Some(node) = self.meta_doc.get(&store::meta_at(&key)) {
            Ok(Some(
                D::deserialize_node(node)
                    .change_context(StorageError::Meta)
                    .attach_meta_node(key.as_str())?,
            ))
        } else {
            Ok(None)
        }
    }

    fn set_meta(&mut self, prefix: &StorePath, meta: &PrefixMeta) -> StorageResult<()> {
        let key = store::meta_key("meta", prefix);
        let at = store::meta_at(&key);
        let node = D::serialize_node(meta, &Noticed::unlimited())
            .change_context(StorageError::Meta)
            .attach_meta_node(key.as_str())?;
        self.meta_doc
            .set(&at, node)
            .change_context(StorageError::Meta)
            .attach_meta_node(key.as_str())?;
        Ok(())
    }

    fn get_schema_snapshots(&self, prefix: &StorePath) -> StorageResult<Vec<SchemaSnapshot>> {
        let key = store::meta_key("schema", prefix);
        match self.meta_doc.get(&store::meta_at(&key)) {
            Some(node) => D::deserialize_node(node)
                .change_context(StorageError::Meta)
                .attach_meta_node(key.as_str()),
            None => Ok(Vec::new()),
        }
    }

    fn set_schema_snapshots(
        &mut self,
        prefix: &StorePath,
        trees: &[SchemaSnapshot],
    ) -> StorageResult<()> {
        let key = store::meta_key("schema", prefix);
        let at = store::meta_at(&key);
        let node = D::serialize_node(trees, &Noticed::unlimited())
            .change_context(StorageError::Meta)
            .attach_meta_node(key.as_str())?;
        self.meta_doc
            .set(&at, node)
            .change_context(StorageError::Meta)
            .attach_meta_node(key.as_str())?;
        Ok(())
    }

    fn get_migration_log(&self, prefix: &StorePath) -> StorageResult<Option<Vec<AppliedStep>>> {
        let key = store::meta_key("log", prefix);
        if let Some(node) = self.meta_doc.get(&store::meta_at(&key)) {
            Ok(Some(
                D::deserialize_node(node)
                    .change_context(StorageError::Meta)
                    .attach_meta_node(key.as_str())?,
            ))
        } else {
            Ok(None)
        }
    }

    fn set_migration_log(&mut self, prefix: &StorePath, log: &[AppliedStep]) -> StorageResult<()> {
        let key = store::meta_key("log", prefix);
        let at = store::meta_at(&key);
        let node = D::serialize_node(&log, &Noticed::unlimited())
            .change_context(StorageError::Meta)
            .attach_meta_node(key.as_str())?;
        self.meta_doc
            .set(&at, node)
            .change_context(StorageError::Meta)
            .attach_meta_node(key.as_str())?;
        Ok(())
    }
}
