use crate::store::CodecFormat;
use crate::store::screening::Noticed;
use crate::store::{Occupied, StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::Report;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

pub trait TextDocument: Send + Sync + Sized + Clone + 'static {
    type Node: Clone + Debug;
    fn format() -> CodecFormat;

    fn get(&self, parts: &[&str]) -> Option<&Self::Node>;
    fn set(&mut self, parts: &[&str], node: Self::Node) -> StorageResult<()>;
    fn delete(&mut self, parts: &[&str]) -> StorageResult<Option<Self::Node>>;
    fn delete_subtree(&mut self, parts: &[&str]) -> StorageResult<()>;
    fn scan(&self, parts: &[&str]) -> StorageResult<Vec<(String, Self::Node)>>;
    fn parse(src: &str) -> StorageResult<Self>;
    fn serialize(&self) -> StorageResult<String>;
    fn empty() -> Self;
    fn deserialize_node<T: DeserializeOwned>(node: &Self::Node) -> StorageResult<T>;
    /// Renders `value` into a node, counting the levels as they go past.
    ///
    /// `depth` is carried into the codec's own pass, so the count sees the
    /// shape the file gets: wrapping the value hands `is_human_readable` to
    /// whichever serializer really runs, and a `Serialize` that branches on it
    /// answers once.
    ///
    /// A refusal comes back as this codec's own error, because that is all a
    /// `Serializer` may return; [`Noticed::overflowed`] is how a caller asks
    /// whether the count was what stopped it.
    fn serialize_node<T: Serialize + ?Sized>(value: &T, seen: &Noticed)
    -> StorageResult<Self::Node>;
    fn node_to_bytes(node: &Self::Node) -> StorageResult<Vec<u8>>;
    fn bytes_to_node(bytes: &[u8]) -> StorageResult<Self::Node>;

    /// Runs `f` against a deserializer over this format's own bytes.
    fn with_bytes_de(
        bytes: &[u8],
        f: &mut dyn FnMut(&mut dyn erased_serde::Deserializer) -> StorageResult<()>,
    ) -> StorageResult<()>;
}

pub trait Navigable: Sized + Clone {
    fn make_empty_map() -> Self;
    fn get_child(&self, key: &str) -> Option<&Self>;
    fn get_child_mut(&mut self, key: &str) -> Option<&mut Self>;
    fn is_map(&self) -> bool;
    fn has_children(&self) -> bool;
    fn insert_child(&mut self, key: &str, val: Self);
    fn remove_child(&mut self, key: &str) -> Option<Self>;
    fn scan_children(&self) -> Vec<(String, Self)>;
}

pub fn generic_get<'a, N: Navigable>(root: &'a N, parts: &[&str]) -> Option<&'a N> {
    if parts.is_empty() {
        return Some(root);
    }

    let mut current = root;
    for part in parts {
        current = current.get_child(part)?;
    }
    Some(current)
}

pub fn generic_set<N: Navigable>(root: &mut N, parts: &[&str], node: N) -> StorageResult<()> {
    if parts.is_empty() {
        *root = node;
        return Ok(());
    }
    let (last, heads) = parts.split_last().unwrap();
    let mut current = root;
    for (at, &part) in heads.iter().enumerate() {
        if !current.is_map() {
            return Err(refused(
                Occupied::Value {
                    level: level(parts, at),
                },
                parts,
            ));
        }
        if current.get_child(part).is_none() {
            current.insert_child(part, N::make_empty_map());
        }
        current = current.get_child_mut(part).unwrap();
    }

    if !current.is_map() {
        return Err(refused(
            Occupied::Value {
                level: level(parts, heads.len()),
            },
            parts,
        ));
    }
    if !node.is_map()
        && let Some(existing) = current.get_child(last)
        && existing.is_map()
        && existing.has_children()
    {
        return Err(refused(
            Occupied::Branch {
                level: level(parts, parts.len()),
            },
            parts,
        ));
    }

    current.insert_child(last, node);
    Ok(())
}

fn level(parts: &[&str], upto: usize) -> String {
    StorePath::from_segments(&parts[..upto])
        .as_str()
        .to_string()
}

fn refused(occupied: Occupied, parts: &[&str]) -> Report<StorageError> {
    let writing = StorePath::from_segments(parts);
    Report::new(occupied)
        .change_context(StorageError::Write)
        .attach(format!("writing: {writing}"))
        .attach("a document holds a value at a level or values under it, never both")
}

pub fn generic_delete<N: Navigable>(root: &mut N, parts: &[&str]) -> StorageResult<Option<N>> {
    if parts.is_empty() {
        return Ok(None);
    }
    let (last, heads) = parts.split_last().unwrap();
    let mut current = root;
    for &part in heads {
        if let Some(next) = current.get_child_mut(part) {
            current = next;
        } else {
            return Ok(None);
        }
    }

    Ok(current.remove_child(last))
}

pub fn generic_delete_subtree<N: Navigable>(root: &mut N, parts: &[&str]) -> StorageResult<()> {
    if parts.is_empty() {
        *root = N::make_empty_map();
        return Ok(());
    }

    let (last, heads) = parts.split_last().unwrap();
    let mut current = root;
    for &part in heads {
        match current.get_child_mut(part) {
            Some(next) => current = next,
            None => return Ok(()),
        }
    }

    current.remove_child(last);
    Ok(())
}

pub fn generic_scan<N: Navigable>(root: &N, parts: &[&str]) -> StorageResult<Vec<(String, N)>> {
    let mut results = Vec::new();
    let prefix = StorePath::from_segments(parts);

    let node = if parts.is_empty() {
        Some(root)
    } else {
        generic_get(root, parts)
    };

    if let Some(node) = node {
        for (k, v) in node.scan_children() {
            match prefix.try_push(&k) {
                Ok(full) => results.push((full.as_str().to_string(), v)),
                Err(_) => tracing::warn!(
                    target: "amethystate",
                    under = %prefix,
                    child = ?k,
                    "a scan passed over a name no path can hold; it stays in the file, \
                     and nothing addressed by a path reaches it",
                ),
            }
        }
    }

    Ok(results)
}
