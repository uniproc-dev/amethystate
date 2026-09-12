use crate::StorageResult;
use crate::codec::CodecError;
use crate::store::backend::text::document::{
    Navigable, TextDocument, generic_delete, generic_delete_subtree, generic_get, generic_scan,
    generic_scan_keys, generic_set, walks_one_node,
};
use crate::store::backend::text::error::TextStoreError;
use crate::store::screening::Noticed;
use crate::store::{CodecFormat, SmolStr, StorageError, StorePath, Stored};
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Clone, Debug)]
pub struct JsonDocument(pub serde_json::Value);

impl Navigable for serde_json::Value {
    fn make_empty_map() -> Self {
        serde_json::Value::Object(serde_json::Map::new())
    }
    fn get_child(&self, key: Stored<'_>) -> Option<&Self> {
        self.get(key.as_str())
    }
    fn get_child_mut(&mut self, key: Stored<'_>) -> Option<&mut Self> {
        self.get_mut(key.as_str())
    }
    fn is_map(&self) -> bool {
        self.is_object()
    }
    fn has_children(&self) -> bool {
        self.as_object().is_some_and(|m| !m.is_empty())
    }
    fn insert_child(&mut self, key: Stored<'_>, val: Self) {
        if let Some(map) = self.as_object_mut() {
            map.insert(key.as_str().to_string(), val);
        }
    }
    fn remove_child(&mut self, key: Stored<'_>) -> Option<Self> {
        self.as_object_mut().and_then(|m| m.remove(key.as_str()))
    }
    fn scan_children(&self) -> Vec<(SmolStr, Self)> {
        let mut results = Vec::new();
        if let Some(obj) = self.as_object() {
            for (k, v) in obj {
                results.push((SmolStr::new(k), v.clone()));
            }
        }
        results
    }

    fn child_names(&self) -> Vec<SmolStr> {
        match self.as_object() {
            Some(obj) => obj.keys().map(SmolStr::new).collect(),
            None => Vec::new(),
        }
    }

    fn each_child(&self) -> impl Iterator<Item = (&str, &Self)> {
        self.as_object()
            .into_iter()
            .flat_map(|obj| obj.iter().map(|(key, node)| (key.as_str(), node)))
    }

    /// Never, because `serde_json::Value` cannot answer the question that is
    /// being asked.
    ///
    /// What a caller wants to know is whether the two encode to the same bytes,
    /// because that is what decides whether an event is reported. `Value`'s own
    /// equality answers something else and answers it wrongly for this purpose
    /// twice over: its maps compare without regard to order while it *renders*
    /// in insertion order, so a formatter that sorts a file's keys changes every
    /// byte and is called no change at all; and `-0.0` equals `0.0` while the
    /// two render differently.
    ///
    /// Saying `false` costs the shortcut and nothing else - the caller falls
    /// back to comparing what they encode to, which is the question. A node the
    /// store owns can answer cheaply *and* rightly, which is the one thing only
    /// an owned node can do here.
    fn known_same(&self, _other: &Self) -> bool {
        false
    }
}

impl TextDocument for JsonDocument {
    type Node = serde_json::Value;

    fn format() -> CodecFormat {
        CodecFormat::Json
    }

    walks_one_node!();

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
