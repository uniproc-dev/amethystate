use crate::StorageResult;
use crate::codec::CodecError;
use crate::store::backend::text::TextStoreError;
use crate::store::backend::text::document::{
    Navigable, TextDocument, generic_delete, generic_delete_subtree, generic_get, generic_scan,
    generic_set,
};
use crate::store::screening::Noticed;
use crate::store::{CodecFormat, StorageError};
use error_stack::{Report, ResultExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Clone, Debug)]
pub struct TomlDocument(pub toml_edit::DocumentMut);

impl Navigable for toml_edit::Item {
    fn make_empty_map() -> Self {
        toml_edit::Item::Table(toml_edit::Table::new())
    }
    fn get_child(&self, key: &str) -> Option<&Self> {
        self.get(key)
    }
    fn get_child_mut(&mut self, key: &str) -> Option<&mut Self> {
        self.as_table_like_mut().and_then(|t| t.get_mut(key))
    }
    fn is_map(&self) -> bool {
        self.as_table_like().is_some()
    }
    fn has_children(&self) -> bool {
        self.as_table_like().is_some_and(|t| !t.is_empty())
    }
    fn insert_child(&mut self, key: &str, val: Self) {
        if let Some(table) = self.as_table_like_mut() {
            table.insert(key, val);
        }
    }
    fn remove_child(&mut self, key: &str) -> Option<Self> {
        self.as_table_like_mut().and_then(|t| t.remove(key))
    }
    fn scan_children(&self) -> Vec<(String, Self)> {
        let mut results = Vec::new();
        if let Some(tbl) = self.as_table_like() {
            for (k, v) in tbl.iter() {
                results.push((k.to_string(), v.clone()));
            }
        }
        results
    }
}

impl TextDocument for TomlDocument {
    type Node = toml_edit::Item;

    fn format() -> CodecFormat {
        CodecFormat::Toml
    }

    fn get(&self, parts: &[&str]) -> Option<&Self::Node> {
        generic_get(self.0.as_item(), parts)
    }

    fn set(&mut self, parts: &[&str], node: Self::Node) -> StorageResult<()> {
        let is_root = parts.is_empty();
        if is_root {
            let table = match node.into_table() {
                Ok(t) => t,
                Err(_) => {
                    return Err(Report::new(TextStoreError::RootMustBeObject)
                        .change_context(StorageError::Write)
                        .attach("the write was addressed at the document root"));
                }
            };
            *self.0.as_item_mut() = toml_edit::Item::Table(table);
            return Ok(());
        }
        generic_set(self.0.as_item_mut(), parts, node)
    }

    fn delete(&mut self, parts: &[&str]) -> StorageResult<Option<Self::Node>> {
        if parts.is_empty() {
            return Ok(None);
        }
        generic_delete(self.0.as_item_mut(), parts)
    }

    fn delete_subtree(&mut self, parts: &[&str]) -> StorageResult<()> {
        generic_delete_subtree(self.0.as_item_mut(), parts)
    }

    fn scan(&self, parts: &[&str]) -> StorageResult<Vec<(String, Self::Node)>> {
        generic_scan(self.0.as_item(), parts)
    }

    fn parse(src: &str) -> StorageResult<Self> {
        let doc = src
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| CodecError::Toml(e.to_string()))
            .map_err(TextStoreError::from)
            .change_context(StorageError::Open)?;

        Ok(TomlDocument(doc))
    }

    fn serialize(&self) -> StorageResult<String> {
        Ok(self.0.to_string())
    }

    fn empty() -> Self {
        TomlDocument(toml_edit::DocumentMut::new())
    }

    fn deserialize_node<T: DeserializeOwned>(node: &Self::Node) -> StorageResult<T> {
        let mut doc = toml_edit::DocumentMut::new();
        doc.as_table_mut().insert("val", node.clone());
        let s = doc.to_string();

        #[derive(serde::Deserialize)]
        struct Unwrap<T> {
            val: T,
        }
        let unwrapped: Unwrap<T> = toml_edit::de::from_str(&s)
            .map_err(|e| CodecError::Toml(e.to_string()))
            .map_err(TextStoreError::from)
            .change_context(StorageError::Codec)
            .attach_with(|| format!("into: {}", std::any::type_name::<T>()))?;

        Ok(unwrapped.val)
    }

    fn serialize_node<T: Serialize + ?Sized>(
        value: &T,
        seen: &Noticed,
    ) -> StorageResult<Self::Node> {
        #[derive(serde::Serialize)]
        struct Wrap<'a, T: ?Sized> {
            val: &'a T,
        }

        let s = toml_edit::ser::to_string(&Wrap {
            val: &seen.count(value),
        })
        .map_err(|e| CodecError::Toml(e.to_string()))
        .map_err(TextStoreError::from)
        .change_context(StorageError::Codec)
        .attach_with(|| format!("from: {}", std::any::type_name::<T>()))?;

        let doc = s
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| CodecError::Toml(e.to_string()))
            .map_err(TextStoreError::from)
            .change_context(StorageError::Codec)
            .attach_with(|| format!("from: {}", std::any::type_name::<T>()))?;
        Ok(doc
            .as_table()
            .get("val")
            .cloned()
            .unwrap_or(toml_edit::Item::None))
    }

    fn with_bytes_de(
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()> {
        let node = Self::bytes_to_node(bytes)?;
        let rendered = match node.as_value() {
            Some(value) => value.to_string(),
            None => match node.clone().into_table() {
                Ok(table) => toml_edit::Value::InlineTable(table.into_inline_table()).to_string(),
                Err(other) => {
                    return Err(Report::new(TextStoreError::Codec(CodecError::Toml(format!(
                        "a {} is not a value and cannot be read as one",
                        other.type_name()
                    ))))
                    .change_context(StorageError::Codec));
                }
            },
        };
        let de: toml_edit::de::ValueDeserializer = rendered
            .trim()
            .parse()
            .map_err(|e: toml_edit::de::Error| {
                TextStoreError::Codec(CodecError::Toml(e.to_string()))
            })
            .change_context(StorageError::Codec)?;
        let mut erased = <dyn erased_serde::Deserializer>::erase(de);
        f(&mut erased)
    }

    fn node_to_bytes(node: &Self::Node) -> StorageResult<Vec<u8>> {
        let mut doc = toml_edit::DocumentMut::new();
        doc.as_table_mut().insert("val", node.clone());
        Ok(doc.to_string().into_bytes())
    }

    fn bytes_to_node(bytes: &[u8]) -> StorageResult<Self::Node> {
        let s = std::str::from_utf8(bytes)
            .map_err(|e| CodecError::Toml(e.to_string()))
            .map_err(TextStoreError::from)
            .change_context(StorageError::Codec)?;

        let doc = s
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| CodecError::Toml(e.to_string()))
            .map_err(TextStoreError::from)
            .change_context(StorageError::Codec)?;

        Ok(doc
            .as_table()
            .get("val")
            .cloned()
            .unwrap_or(toml_edit::Item::None))
    }
}
