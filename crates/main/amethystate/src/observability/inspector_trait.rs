use crate::StorageResult;
use crate::store::CodecFormat;
use crate::store::meta::SchemaSnapshot;
use amethystate_core::path::StorePath;

pub trait InspectorBackend {
    fn format(&self) -> CodecFormat;
    fn scan_all(&self) -> StorageResult<Vec<(StorePath, Vec<u8>)>>;
    fn get_schema_snapshots(&self) -> StorageResult<Vec<(String, SchemaSnapshot)>>;
    fn set_raw(&mut self, key: &str, value: &[u8]) -> StorageResult<()>;
}
