use crate::StorageResult;
use crate::store::CodecFormat;
use crate::store::InspectorBackend;
use crate::store::StorageError;
use crate::store::backend::text::store::scan_prefix_impl;
use crate::store::backend::text::{TextDocument, TextStore};
use crate::store::facts::Facts;
use crate::store::meta::SchemaSnapshot;
use amethystate_core::path::StorePath;
use error_stack::ResultExt;

impl<D: TextDocument + Send + 'static> InspectorBackend for TextStore<D> {
    fn format(&self) -> CodecFormat {
        D::format()
    }

    fn scan_all(&self) -> StorageResult<Vec<(StorePath, Vec<u8>)>> {
        let declared = self.inner.declared()?;
        let guard = self.inner.files.data.doc.read();

        scan_prefix_impl(&*guard, &StorePath::root(), &declared)
            .attach_store_file(&self.inner.files.data.path)
    }

    fn get_schema_snapshots(&self) -> StorageResult<Vec<(String, SchemaSnapshot)>> {
        Ok(self
            .inner
            .recorded_schemas()?
            .into_iter()
            .map(|(prefix, snapshot)| (prefix.to_string(), snapshot))
            .collect())
    }

    fn set_raw(&mut self, key: &str, value: &[u8]) -> StorageResult<()> {
        self.inner.check_debouncer()?;
        let path = StorePath::parse_joined(key)
            .change_context(StorageError::Path)
            .attach_raw_key(key)?;

        let node = D::bytes_to_node(value)
            .change_context(StorageError::Write)
            .attach_store_file(&self.inner.files.data.path)
            .attach_key(&path)
            .attach_value_bytes(value.len())?;

        self.inner.set_node(path, node, None)
    }
}
