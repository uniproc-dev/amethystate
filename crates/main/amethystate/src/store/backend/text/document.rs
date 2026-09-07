use crate::store::CodecFormat;
use crate::store::screening::Noticed;
use crate::store::{Occupied, StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::Report;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;

pub trait TextDocument: Send + Sync + Sized + Clone + 'static {
    type Node: Clone + Debug + Navigable;
    fn format() -> CodecFormat;

    fn get(&self, at: &StorePath) -> Option<&Self::Node>;
    fn set(&mut self, at: &StorePath, node: Self::Node) -> StorageResult<()>;
    fn delete(&mut self, at: &StorePath) -> StorageResult<Option<Self::Node>>;
    fn delete_subtree(&mut self, at: &StorePath) -> StorageResult<()>;
    fn scan(&self, prefix: &StorePath) -> StorageResult<Vec<(StorePath, Self::Node)>>;
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
    fn serialize_node<T: Serialize + ?Sized>(
        value: &T,
        seen: &Noticed,
    ) -> StorageResult<Self::Node>;
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

pub fn generic_get<'a, N: Navigable>(root: &'a N, at: &StorePath) -> Option<&'a N> {
    let mut current = root;

    for name in at.segments() {
        current = current.get_child(&name)?;
    }

    Some(current)
}

pub fn generic_set<N: Navigable>(root: &mut N, at: &StorePath, node: N) -> StorageResult<()> {
    let Some(last) = at.name() else {
        *root = node;
        return Ok(());
    };

    let heads = at.len() - 1;
    let mut current = root;

    for (depth, name) in at.segments().take(heads).enumerate() {
        if !current.is_map() {
            return Err(refused(
                Occupied::Value {
                    level: level(at, depth),
                },
                at,
            ));
        }
        if current.get_child(&name).is_none() {
            current.insert_child(&name, N::make_empty_map());
        }
        current = current.get_child_mut(&name).expect("just inserted");
    }

    if !current.is_map() {
        return Err(refused(
            Occupied::Value {
                level: level(at, heads),
            },
            at,
        ));
    }
    if !node.is_map()
        && let Some(existing) = current.get_child(&last)
        && existing.is_map()
        && existing.has_children()
    {
        return Err(refused(
            Occupied::Branch {
                level: level(at, at.len()),
            },
            at,
        ));
    }

    current.insert_child(&last, node);
    Ok(())
}

fn level(at: &StorePath, upto: usize) -> String {
    StorePath::from_segments(at.segments().take(upto))
        .as_str()
        .to_string()
}

fn refused(occupied: Occupied, writing: &StorePath) -> Report<StorageError> {
    Report::new(occupied)
        .change_context(StorageError::Write)
        .attach(format!("writing: {writing}"))
        .attach("a document holds a value at a level or values under it, never both")
}

/// Removes the value at `parts`, and every level above it that held nothing
/// else.
///
/// A flat engine has no node above a key at all, so deleting the only thing
/// under `a` leaves no `a`. A document has one, and leaving it behind as `{}`
/// is a difference a caller can see: a scan lists it, and writing a value and
/// deleting it stops being a round trip.
///
/// Only a level this delete emptied is pruned, so a map written as `{}` and
/// never added to is left alone - nothing was removed from it, and there is
/// nothing to walk back up.
pub fn generic_delete<N: Navigable>(root: &mut N, at: &StorePath) -> StorageResult<Option<N>> {
    let Some(last) = at.name() else {
        return Ok(None);
    };

    let heads = at.len() - 1;
    let mut current = &mut *root;
    for name in at.segments().take(heads) {
        match current.get_child_mut(&name) {
            Some(next) => current = next,
            None => return Ok(None),
        }
    }

    let removed = current.remove_child(&last);
    if removed.is_some() {
        prune_empty_above(root, at, heads);
    }

    Ok(removed)
}

/// Walks back up the levels above the delete, dropping each one it left empty.
///
/// Deepest first, so a chain of levels that existed only to hold the value
/// goes with it rather than one link of it.
fn prune_empty_above<N: Navigable>(root: &mut N, at: &StorePath, heads: usize) {
    for depth in (1..=heads).rev() {
        let name = at.segment_at(depth - 1).expect("above the delete");

        let mut current = &mut *root;
        for above in at.segments().take(depth - 1) {
            match current.get_child_mut(&above) {
                Some(next) => current = next,
                None => return,
            }
        }

        match current.get_child(&name) {
            Some(node) if node.is_map() && !node.has_children() => current.remove_child(&name),
            _ => return,
        };
    }
}

pub fn generic_delete_subtree<N: Navigable>(root: &mut N, at: &StorePath) -> StorageResult<()> {
    let Some(last) = at.name() else {
        *root = N::make_empty_map();
        return Ok(());
    };

    let heads = at.len() - 1;
    let mut current = root;
    for name in at.segments().take(heads) {
        match current.get_child_mut(&name) {
            Some(next) => current = next,
            None => return Ok(()),
        }
    }

    current.remove_child(&last);
    Ok(())
}

pub fn generic_scan<N: Navigable>(
    root: &N,
    prefix: &StorePath,
) -> StorageResult<Vec<(StorePath, N)>> {
    let mut results = Vec::new();

    if let Some(node) = generic_get(root, prefix) {
        for (k, v) in node.scan_children() {
            match prefix.try_push(&k) {
                Ok(full) => results.push((full, v)),
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
