use crate::StorageResult;
use crate::codec::CodecError;
use crate::store::backend::text::document::{
    Navigable, TextDocument, generic_delete, generic_delete_subtree, generic_get, generic_scan,
    generic_scan_keys, generic_set,
};
use crate::store::backend::text::error::TextStoreError;
use crate::store::backend::text::tree::Node;
use crate::store::screening::Noticed;
use crate::store::{CodecFormat, StorageError, StorePath};
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// The json document over a node the store owns.
///
/// Reads the same files [`JsonDocument`](super::json_doc::JsonDocument) does
/// and answers the same paths; the two stand side by side so what owning the
/// node is worth can be measured rather than argued.
#[derive(Clone, Debug)]
pub struct JsonTree(pub Node);

fn codec(why: serde_json::Error) -> Report<TextStoreError> {
    Report::new(TextStoreError::Codec(CodecError::Json(why)))
}

impl TextDocument for JsonTree {
    type Node = Node;

    fn format() -> CodecFormat {
        CodecFormat::Json
    }

    fn get(&self, at: &StorePath) -> Option<&Self::Node> {
        generic_get(&self.0, at)
    }

    fn set(&mut self, at: &StorePath, node: Self::Node) -> StorageResult<()> {
        if at.is_root() {
            if !node.is_map() {
                return Err(Report::new(TextStoreError::RootMustBeObject)
                    .change_context(StorageError::Write)
                    .attach("the write was addressed at the document root"));
            }
            self.0 = node;
            return Ok(());
        }
        generic_set(&mut self.0, at, node)
    }

    fn delete(&mut self, at: &StorePath) -> StorageResult<Option<Self::Node>> {
        generic_delete(&mut self.0, at)
    }

    fn delete_subtree(&mut self, at: &StorePath) -> StorageResult<()> {
        generic_delete_subtree(&mut self.0, at)
    }

    fn scan(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Self::Node)>> {
        generic_scan(&self.0, prefix)
    }

    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>> {
        generic_scan_keys(&self.0, prefix)
    }

    fn parse(src: &str) -> StorageResult<Self> {
        let node: Node = serde_json::from_str(src)
            .map_err(codec)
            .change_context(StorageError::Open)?;

        if !node.is_map() {
            return Err(
                Report::new(TextStoreError::RootMustBeObject).change_context(StorageError::Open)
            );
        }
        Ok(JsonTree(node))
    }

    fn serialize(&self) -> StorageResult<String> {
        serde_json::to_string_pretty(&self.0)
            .map_err(codec)
            .change_context(StorageError::Flush)
    }

    fn empty() -> Self {
        JsonTree(Node::make_empty_map())
    }

    fn deserialize_node<T: DeserializeOwned>(node: &Self::Node) -> StorageResult<T> {
        let bytes = Self::node_to_bytes(node)?;

        serde_json::from_slice(&bytes)
            .map_err(codec)
            .change_context(StorageError::Codec)
            .attach_with(|| format!("into: {}", std::any::type_name::<T>()))
    }

    fn serialize_node<T: Serialize + ?Sized>(
        value: &T,
        seen: &Noticed,
    ) -> StorageResult<Self::Node> {
        let bytes = serde_json::to_vec(&seen.count(value))
            .map_err(codec)
            .change_context(StorageError::Codec)
            .attach_with(|| format!("from: {}", std::any::type_name::<T>()))?;

        Self::bytes_to_node(&bytes)
    }

    fn with_bytes_de(
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()> {
        let mut de = serde_json::Deserializer::from_slice(bytes);
        let mut erased = <dyn erased_serde::Deserializer>::erase(&mut de);
        f(&mut erased)
    }

    fn node_to_bytes(node: &Self::Node) -> StorageResult<Vec<u8>> {
        serde_json::to_vec(node)
            .map_err(codec)
            .change_context(StorageError::Codec)
    }

    fn bytes_to_node(bytes: &[u8]) -> StorageResult<Self::Node> {
        serde_json::from_slice(bytes)
            .map_err(codec)
            .change_context(StorageError::Codec)
    }
}
