use crate::StorageResult;
use crate::store::CodecFormat;
use crate::store::InspectorBackend;
use crate::store::meta::SchemaSnapshot;
use crate::stores::TomlStore;

impl InspectorBackend for TomlStore {
    fn format(&self) -> CodecFormat {
        self.0.format()
    }
    fn scan_all(&self) -> StorageResult<Vec<(amethystate_core::path::StorePath, Vec<u8>)>> {
        self.0.scan_all()
    }
    fn get_schema_snapshots(&self) -> StorageResult<Vec<(String, SchemaSnapshot)>> {
        self.0.get_schema_snapshots()
    }
    fn set_raw(&mut self, key: &str, value: &[u8]) -> StorageResult<()> {
        self.0.set_raw(key, value)
    }
}
