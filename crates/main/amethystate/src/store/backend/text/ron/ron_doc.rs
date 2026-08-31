use crate::StorageResult;
use crate::codec::CodecError;
use crate::store::backend::text::document::{
    Navigable, TextDocument, generic_delete, generic_delete_subtree, generic_get, generic_scan,
    generic_set,
};
use crate::store::backend::text::error::TextStoreError;
use crate::store::screening::Noticed;
use crate::store::{CodecFormat, StorageError};
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Clone, Debug)]
pub struct RonDocument(pub ::ron::value::Value);

impl Navigable for ::ron::value::Value {
    fn make_empty_map() -> Self {
        ::ron::value::Value::Map(::ron::value::Map::new())
    }
    fn get_child(&self, key: &str) -> Option<&Self> {
        if let ::ron::value::Value::Map(map) = self {
            map.get(&::ron::value::Value::String(key.to_string()))
        } else {
            None
        }
    }
    fn get_child_mut(&mut self, key: &str) -> Option<&mut Self> {
        if let ::ron::value::Value::Map(map) = self {
            map.get_mut(&::ron::value::Value::String(key.to_string()))
        } else {
            None
        }
    }
    fn is_map(&self) -> bool {
        matches!(self, ::ron::value::Value::Map(_))
    }
    fn has_children(&self) -> bool {
        matches!(self, ::ron::value::Value::Map(map) if !map.is_empty())
    }
    fn insert_child(&mut self, key: &str, val: Self) {
        if let ::ron::value::Value::Map(map) = self {
            map.insert(::ron::value::Value::String(key.to_string()), val);
        }
    }
    fn remove_child(&mut self, key: &str) -> Option<Self> {
        if let ::ron::value::Value::Map(map) = self {
            map.remove(&::ron::value::Value::String(key.to_string()))
        } else {
            None
        }
    }
    fn scan_children(&self) -> Vec<(String, Self)> {
        let mut results = Vec::new();
        if let ::ron::value::Value::Map(map) = self {
            for (k, v) in map.iter() {
                if let ::ron::value::Value::String(s) = k {
                    results.push((s.clone(), v.clone()));
                }
            }
        }
        results
    }
}

impl TextDocument for RonDocument {
    type Node = ::ron::value::Value;

    fn format() -> CodecFormat {
        CodecFormat::Ron
    }

    fn get(&self, parts: &[&str]) -> Option<&Self::Node> {
        generic_get(&self.0, parts)
    }

    fn set(&mut self, parts: &[&str], node: Self::Node) -> StorageResult<()> {
        let is_root = parts.is_empty();
        if is_root {
            if !matches!(node, ::ron::value::Value::Map(_)) {
                return Err(Report::new(TextStoreError::RootMustBeObject)
                    .change_context(StorageError::Write)
                    .attach("the write was addressed at the document root"));
            }
            self.0 = node;
            return Ok(());
        }
        generic_set(&mut self.0, parts, node)
    }

    fn delete(&mut self, parts: &[&str]) -> StorageResult<Option<Self::Node>> {
        generic_delete(&mut self.0, parts)
    }

    fn delete_subtree(&mut self, parts: &[&str]) -> StorageResult<()> {
        generic_delete_subtree(&mut self.0, parts)
    }

    fn scan(&self, parts: &[&str]) -> StorageResult<Vec<(String, Self::Node)>> {
        generic_scan(&self.0, parts)
    }

    fn parse(src: &str) -> StorageResult<Self> {
        let val: ::ron::value::Value = ::ron::from_str(src)
            .map_err(|e| TextStoreError::Codec(CodecError::Ron(e.into())))
            .change_context(StorageError::Open)?;

        if !matches!(val, ::ron::value::Value::Map(_)) {
            return Err(
                Report::new(TextStoreError::RootMustBeObject).change_context(StorageError::Open)
            );
        }
        Ok(RonDocument(val))
    }

    fn serialize(&self) -> StorageResult<String> {
        ::ron::ser::to_string_pretty(&self.0, ::ron::ser::PrettyConfig::default())
            .map_err(|e| TextStoreError::Codec(CodecError::Ron(e)))
            .change_context(StorageError::Flush)
    }

    fn empty() -> Self {
        RonDocument(::ron::value::Value::Map(::ron::value::Map::new()))
    }

    fn deserialize_node<T: DeserializeOwned>(node: &Self::Node) -> StorageResult<T> {
        node.clone()
            .into_rust::<T>()
            .map_err(|e| TextStoreError::Codec(CodecError::Ron(e)))
            .change_context(StorageError::Codec)
            .attach_with(|| format!("into: {}", std::any::type_name::<T>()))
    }

    fn serialize_node<T: Serialize + ?Sized>(
        value: &T,
        seen: &Noticed,
    ) -> StorageResult<Self::Node> {
        let s = ::ron::ser::to_string(&seen.count(value))
            .map_err(|e| TextStoreError::Codec(CodecError::Ron(e)))
            .change_context(StorageError::Codec)
            .attach_with(|| format!("from: {}", std::any::type_name::<T>()))?;
        let node: ::ron::value::Value = ::ron::from_str(&s)
            .map_err(|e| TextStoreError::Codec(CodecError::Ron(e.into())))
            .change_context(StorageError::Codec)
            .attach_with(|| format!("from: {}", std::any::type_name::<T>()))?;
        Ok(node)
    }

    fn with_bytes_de(
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()> {
        let node = Self::bytes_to_node(bytes)?;
        let mut erased = <dyn erased_serde::Deserializer>::erase(node);
        f(&mut erased)
    }

    fn node_to_bytes(node: &Self::Node) -> StorageResult<Vec<u8>> {
        let s = ::ron::ser::to_string(node)
            .map_err(|e| TextStoreError::Codec(CodecError::Ron(e)))
            .change_context(StorageError::Codec)?;
        Ok(s.into_bytes())
    }

    fn bytes_to_node(bytes: &[u8]) -> StorageResult<Self::Node> {
        let s = std::str::from_utf8(bytes)
            .map_err(|e| CodecError::Custom(e.to_string()))
            .map_err(TextStoreError::from)
            .change_context(StorageError::Codec)?;
        let node: ::ron::value::Value = ::ron::from_str(s)
            .map_err(|e| TextStoreError::Codec(CodecError::Ron(e.into())))
            .change_context(StorageError::Codec)?;
        Ok(node)
    }
}
