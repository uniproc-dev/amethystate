//! Reading a store from outside the program that wrote it.
//!
//! A tool opens a file it did not create, holds no types for what is in it,
//! and cannot ask an `#[amethystate]` struct anything: the declarations live
//! in somebody else's binary. So everything here answers from what is on the
//! disk - the bytes, and the snapshot the store recorded beside them.
//!
//! Which is why it sits with the engines rather than with the reporting an
//! application does about itself. The engines implement it, and what an
//! application shows about its own fields it shows through
//! [`observability`](crate::observability), where the types are in hand.

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
