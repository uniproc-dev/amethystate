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
pub struct RonDocument(pub ::ron::value::Value);

impl Navigable for ::ron::value::Value {
    fn make_empty_map() -> Self {
        ::ron::value::Value::Map(::ron::value::Map::new())
    }
    fn get_child(&self, key: Stored<'_>) -> Option<&Self> {
        if let ::ron::value::Value::Map(map) = self {
            map.get(&::ron::value::Value::String(key.as_str().to_string()))
        } else {
            None
        }
    }
    fn get_child_mut(&mut self, key: Stored<'_>) -> Option<&mut Self> {
        if let ::ron::value::Value::Map(map) = self {
            map.get_mut(&::ron::value::Value::String(key.as_str().to_string()))
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
    fn insert_child(&mut self, key: Stored<'_>, val: Self) {
        if let ::ron::value::Value::Map(map) = self {
            map.insert(::ron::value::Value::String(key.as_str().to_string()), val);
        }
    }
    fn remove_child(&mut self, key: Stored<'_>) -> Option<Self> {
        if let ::ron::value::Value::Map(map) = self {
            map.remove(&::ron::value::Value::String(key.as_str().to_string()))
        } else {
            None
        }
    }
    fn scan_children(&self) -> Vec<(SmolStr, Self)> {
        let mut results = Vec::new();
        if let ::ron::value::Value::Map(map) = self {
            for (k, v) in map.iter() {
                if let ::ron::value::Value::String(s) = k {
                    results.push((SmolStr::new(s), v.clone()));
                }
            }
        }
        results
    }

    fn child_names(&self) -> Vec<SmolStr> {
        let mut results = Vec::new();
        if let ::ron::value::Value::Map(map) = self {
            for (k, _) in map.iter() {
                if let ::ron::value::Value::String(s) = k {
                    results.push(SmolStr::new(s));
                }
            }
        }
        results
    }

    fn each_child(&self) -> impl Iterator<Item = (&str, &Self)> {
        let map = match self {
            ::ron::value::Value::Map(map) => Some(map),
            _ => None,
        };

        map.into_iter().flat_map(|map| {
            map.iter().filter_map(|(key, node)| match key {
                ::ron::value::Value::String(name) => Some((name.as_str(), node)),
                _ => None,
            })
        })
    }

    /// Never, for the same reason as json: `ron::value::Value`'s equality is not
    /// "these encode alike", and here that is the only question worth asking.
    fn known_same(&self, _other: &Self) -> bool {
        false
    }
}

impl TextDocument for RonDocument {
    type Node = ::ron::value::Value;

    fn format() -> CodecFormat {
        CodecFormat::Ron
    }

    walks_one_node!();

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
