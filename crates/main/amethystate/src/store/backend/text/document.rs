use crate::store::CodecFormat;
use crate::store::screening::Noticed;
use crate::store::{Occupied, StorageError, StorageResult};
use amethystate_core::path::StorePath;
use amethystate_core::path::{SmolStr, Stored};
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

    /// The paths one level under `prefix`, without reading what is at them.
    fn scan_keys(&self, prefix: &StorePath) -> StorageResult<Vec<StorePath>>;
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

/// How a document's node is walked, whatever library the node comes from.
///
/// A child is addressed by [`Stored`] rather than by a `&str`, because a
/// document holds two different things under names that look alike: a level of
/// a declared tree, held under its own name, and a key of the plane, held under
/// the whole path's spelling. Which one a caller means is the layout's decision
/// and belongs in the type, not in whoever happens to read the call.
pub trait Navigable: Sized + Clone {
    fn make_empty_map() -> Self;
    fn get_child(&self, key: Stored<'_>) -> Option<&Self>;
    fn get_child_mut(&mut self, key: Stored<'_>) -> Option<&mut Self>;
    fn is_map(&self) -> bool;
    fn has_children(&self) -> bool;
    fn insert_child(&mut self, key: Stored<'_>, val: Self);
    fn remove_child(&mut self, key: Stored<'_>) -> Option<Self>;

    /// The children, each with the name it is stored under.
    ///
    /// The name comes back in the form a scan hands straight to
    /// [`StorePath::try_push_shared`], so a node that already holds its names
    /// that way gives one over without copying it.
    fn scan_children(&self) -> Vec<(SmolStr, Self)>;

    /// The names alone, for a walk that is looking for paths.
    ///
    /// Which is most of them: the scans that find what a document holds read
    /// every value out only to drop it, and on a node that owns a subtree that
    /// is a deep copy per key.
    fn child_names(&self) -> Vec<SmolStr>;

    /// The children, borrowed, each under the name the node holds it by, in the
    /// order the node holds them.
    ///
    /// For a walk down two documents at once: it wants to compare a child
    /// against the child of the same name and only then decide whether to build
    /// a path for it. [`scan_children`](Self::scan_children) copies the values
    /// and a scan builds a path per name, which for a level whose children
    /// mostly match is all of the work and none of the answer.
    ///
    /// Handed over one at a time, and borrowed, because a walk down two levels
    /// at once reads each name once and keeps almost none of them: a level of a
    /// hundred thousand entries where one moved should pay for the one. Every
    /// format this library reads holds a level in an order it can iterate, so
    /// there is nothing to build here.
    fn each_child(&self) -> impl Iterator<Item = (&str, &Self)>;

    /// The same, gathered.
    fn children(&self) -> Vec<(&str, &Self)> {
        self.each_child().collect()
    }

    /// Whether these two are certainly the same, without encoding either.
    ///
    /// Conservative on purpose: `false` means *not known to be the same*, not
    /// *different*. A node a format's library gives no way to compare answers
    /// `false`, and whoever asked falls back to comparing what the two encode
    /// to - which is what it did before there was anything to ask.
    ///
    /// It is asked where the answer is usually yes: a diff between two readings
    /// of a file compares every key of both, and the reading that found nothing
    /// changed is the one a watcher makes nearly every time.
    fn known_same(&self, other: &Self) -> bool;
}

/// Writes the six methods a document whose root *is* one owned node answers
/// the same way: the walk is `generic_*` over `self.0`.
///
/// What is left to each document is what only it can say - how its text parses
/// and renders, what a node of its own is worth. The one rule shared here is
/// the root: a write addressed at it must be a map, because a scalar there is
/// a document with no keys and every later write has nowhere to go.
///
/// `toml` is not one of these. Its root is a `Document` rather than a node, so
/// every method reaches through `as_item`, and a root write has to become a
/// `Table` rather than merely be one.
macro_rules! walks_one_node {
    () => {
        fn get(&self, at: &StorePath) -> Option<&Self::Node> {
            generic_get(&self.0, at)
        }

        fn set(&mut self, at: &StorePath, node: Self::Node) -> StorageResult<()> {
            if at.is_root() {
                if !Navigable::is_map(&node) {
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
    };
}

pub(crate) use walks_one_node;

pub fn generic_get<'a, N: Navigable>(root: &'a N, at: &StorePath) -> Option<&'a N> {
    let mut current = root;

    for name in at.segments() {
        current = current.get_child(Stored::level(&name))?;
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
        if current.get_child(Stored::level(&name)).is_none() {
            current.insert_child(Stored::level(&name), N::make_empty_map());
        }
        current = current
            .get_child_mut(Stored::level(&name))
            .expect("just inserted");
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
        && let Some(existing) = current.get_child(Stored::level(&last))
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

    current.insert_child(Stored::level(&last), node);
    Ok(())
}

fn level(at: &StorePath, upto: usize) -> String {
    StorePath::from_segments(at.segments().take(upto)).to_string()
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
        match current.get_child_mut(Stored::level(&name)) {
            Some(next) => current = next,
            None => return Ok(None),
        }
    }

    let removed = current.remove_child(Stored::level(&last));
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
            match current.get_child_mut(Stored::level(&above)) {
                Some(next) => current = next,
                None => return,
            }
        }

        match current.get_child(Stored::level(&name)) {
            Some(node) if node.is_map() && !node.has_children() => {
                current.remove_child(Stored::level(&name))
            }
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
        match current.get_child_mut(Stored::level(&name)) {
            Some(next) => current = next,
            None => return Ok(()),
        }
    }

    current.remove_child(Stored::level(&last));
    Ok(())
}

pub fn generic_scan<N: Navigable>(
    root: &N,
    prefix: &StorePath,
) -> StorageResult<Vec<(StorePath, N)>> {
    let mut results = Vec::new();

    if let Some(node) = generic_get(root, prefix) {
        for (k, v) in node.scan_children() {
            results.push((prefix.push_shared(k), v));
        }
    }

    Ok(results)
}

/// [`generic_scan`] for a caller that wants the paths and not the values.
pub fn generic_scan_keys<N: Navigable>(
    root: &N,
    prefix: &StorePath,
) -> StorageResult<Vec<StorePath>> {
    let mut results = Vec::new();

    if let Some(node) = generic_get(root, prefix) {
        for name in node.child_names() {
            results.push(prefix.push_shared(name));
        }
    }

    Ok(results)
}

