//! Where in the file a path is written.

use super::document::{Navigable, TextDocument};
use crate::store::declared::Declared;
use crate::store::facts::Facts;
use crate::store::{StorageError, StorageResult};
use amethystate_core::path::{Level, PathRef, StorePath, Stored};
use error_stack::{Report, ResultExt};

/// Where `path` is written in `doc`, as a path the writers can address by.
pub(super) fn levels<D: TextDocument>(doc: &D, declared: &Declared, path: &StorePath) -> StorePath {
    if path.is_root() || declared.covers(path) {
        return path.clone();
    }

    plane_name(doc, path)
}

/// The node `path` is written at, without building a path to find it by.
pub(super) fn node_at<'a, D: TextDocument>(
    doc: &'a D,
    declared: &Declared,
    path: &StorePath,
) -> Option<&'a D::Node> {
    if path.is_root() || declared.covers(path) {
        return doc.get(path);
    }

    let root = doc.get(&StorePath::root())?;

    if let Some(node) = root.get_child(Stored::whole(path.into())) {
        return Some(node);
    }

    root.get_child(Stored::level(&bare_name(path)?))
}

/// The name the plane holds `path` under: the spelling this library writes, or
/// the bare name where the file already had one.
fn plane_name<D: TextDocument>(doc: &D, path: &StorePath) -> StorePath {
    let Some(root) = doc.get(&StorePath::root()) else {
        return path.as_one_level();
    };

    if root.get_child(Stored::whole(path.into())).is_some() {
        return path.as_one_level();
    }

    match bare_name(path) {
        Some(bare) if root.get_child(Stored::level(&bare)).is_some() => StorePath::segment(bare),
        _ => path.as_one_level(),
    }
}

/// The one level `path` is, unescaped - `None` unless the two spellings differ.
fn bare_name(path: &StorePath) -> Option<Level<'static>> {
    let name = path.name().filter(|_| path.len() == 1)?;

    let held = PathRef::from(path);

    (name != *held.as_str() && StorePath::parse_joined(name.as_str()).is_err())
        .then(|| name.into_owned())
}

/// The path a name at the root stands for, and which of the two it is.
pub(super) fn at_root(declared: &Declared, key: &StorePath) -> StorageResult<(StorePath, Root)> {
    let name = key
        .name()
        .ok_or_else(|| Report::new(StorageError::Path))
        .attach_key(key)
        .attach("a document's root handed back a level with no name")?;

    // A name that reads as the root is the one path a plane key cannot spell:
    // the root's spelling is the empty string, so a member named with nothing
    // would stand for the whole document. Read as the one level it is instead,
    // which is what it looks like in the file - and nothing addresses it,
    // because nothing can spell it either.
    let path = match StorePath::parse_joined(name.as_str()) {
        Ok(path) if !path.is_root() => path,
        _ => key.clone(),
    };

    let root = match path.len() == 1 && declared.covers(&path) {
        true => Root::Tree,
        false => Root::Plane,
    };

    Ok((path, root))
}

/// Which of a document's two parts a name at the root belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Root {
    /// A declared prefix's outermost level, holding a tree.
    Tree,

    /// A whole key, standing for the path it spells.
    Plane,
}
