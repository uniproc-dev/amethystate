use crate::StorageResult;
use crate::codec::CodecError;
use crate::store::backend::text::document::{
    Navigable, TextDocument, generic_delete, generic_delete_subtree, generic_get, generic_scan,
    generic_scan_keys, generic_set,
};
use crate::store::backend::text::error::TextStoreError;
use crate::store::screening::Noticed;
use crate::store::{CodecFormat, StorageError, StorePath};
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Clone, Debug)]
pub struct JsonDocument(pub serde_json::Value);

impl Navigable for serde_json::Value {
    fn make_empty_map() -> Self {
        serde_json::Value::Object(serde_json::Map::new())
    }
    fn get_child(&self, key: crate::store::Stored<'_>) -> Option<&Self> {
        self.get(key.as_str())
    }
    fn get_child_mut(&mut self, key: crate::store::Stored<'_>) -> Option<&mut Self> {
        self.get_mut(key.as_str())
    }
    fn is_map(&self) -> bool {
        self.is_object()
    }
    fn has_children(&self) -> bool {
        self.as_object().is_some_and(|m| !m.is_empty())
    }
    fn insert_child(&mut self, key: crate::store::Stored<'_>, val: Self) {
        if let Some(map) = self.as_object_mut() {
            map.insert(key.as_str().to_string(), val);
        }
    }
    fn remove_child(&mut self, key: crate::store::Stored<'_>) -> Option<Self> {
        self.as_object_mut().and_then(|m| m.remove(key.as_str()))
    }
    fn scan_children(&self) -> Vec<(crate::store::SmolStr, Self)> {
        let mut results = Vec::new();
        if let Some(obj) = self.as_object() {
            for (k, v) in obj {
                results.push((crate::store::SmolStr::new(k), v.clone()));
            }
        }
        results
    }

    fn child_names(&self) -> Vec<crate::store::SmolStr> {
        match self.as_object() {
            Some(obj) => obj.keys().map(crate::store::SmolStr::new).collect(),
            None => Vec::new(),
        }
    }
}

impl TextDocument for JsonDocument {
    type Node = serde_json::Value;

    fn format() -> CodecFormat {
        CodecFormat::Json
    }

    fn get(&self, at: &StorePath) -> Option<&Self::Node> {
        generic_get(&self.0, at)
    }

    fn set(&mut self, at: &StorePath, node: Self::Node) -> StorageResult<()> {
        if at.is_root() {
            if !node.is_object() {
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
        let val: serde_json::Value = serde_json::from_str(src)
            .map_err(|e| TextStoreError::Codec(CodecError::Json(e)))
            .change_context(StorageError::Open)?;

        if !val.is_object() {
            return Err(
                Report::new(TextStoreError::RootMustBeObject).change_context(StorageError::Open)
            );
        }
        Ok(JsonDocument(val))
    }

    fn serialize(&self) -> StorageResult<String> {
        serde_json::to_string_pretty(&self.0)
            .map_err(|e| TextStoreError::Codec(CodecError::Json(e)))
            .change_context(StorageError::Flush)
    }

    fn empty() -> Self {
        JsonDocument(serde_json::Value::Object(serde_json::Map::new()))
    }

    fn deserialize_node<T: DeserializeOwned>(node: &Self::Node) -> StorageResult<T> {
        serde_json::from_value(node.clone())
            .map_err(|e| TextStoreError::Codec(CodecError::Json(e)))
            .change_context(StorageError::Codec)
            .attach_with(|| format!("into: {}", std::any::type_name::<T>()))
    }

    fn serialize_node<T: Serialize + ?Sized>(
        value: &T,
        seen: &Noticed,
    ) -> StorageResult<Self::Node> {
        serde_json::to_value(seen.count(value))
            .map_err(|e| TextStoreError::Codec(CodecError::Json(e)))
            .change_context(StorageError::Codec)
            .attach_with(|| format!("from: {}", std::any::type_name::<T>()))
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
            .map_err(|e| TextStoreError::Codec(CodecError::Json(e)))
            .change_context(StorageError::Codec)
    }

    fn bytes_to_node(bytes: &[u8]) -> StorageResult<Self::Node> {
        serde_json::from_slice(bytes)
            .map_err(|e| TextStoreError::Codec(CodecError::Json(e)))
            .change_context(StorageError::Codec)
    }
}
