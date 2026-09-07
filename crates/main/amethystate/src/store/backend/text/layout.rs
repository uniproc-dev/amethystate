//! Where in the file a path is written.
//!
//! A document holds two things that are not the same kind of thing, and it
//! holds them differently.
//!
//! **The tree** is what a schema declares. `ui.width` is written as `width`
//! inside `ui`, which is why a settings file is worth opening in an editor at
//! all - and it can be read as a tree because the declarations say where each
//! value ends. See [`Declared::holds`].
//!
//! **The plane** is everything else. A path nothing declares is written whole,
//! as one name at the root: `"widths.left.px"`, separator escaped inside the
//! names it came from. Nothing says where the levels of such a path would end
//! and its value would begin, so the file does not pretend to know - and a key
//! written whole needs nobody to tell it apart from a level, because there are
//! no levels down there.
//!
//! Which is exactly how a flat engine holds every key it has, and why the
//! question a document could not answer stops being asked rather than being
//! guessed at. The metadata file is laid out this way for the same reason: see
//! [`meta_key`](super::store::meta_key).
//!
//! # A name that spells no path
//!
//! The plane's names are joined paths, so a name is read as one. `.` is not:
//! it is a level with no name, and neither is `a\`, which ends on an escape
//! holding nothing. A name like that was put in the file by hand, and there is
//! no second reading to weigh - it is one level called that, and it is read as
//! one.
//!
//! Read, and not rewritten. The library spells that same level `\.`, but a file
//! a person wrote is theirs: a save puts the value back under the name they
//! used, and only a path that is new to the file gets the spelling this library
//! would have chosen.

use super::document::TextDocument;
use crate::store::declared::Declared;
use crate::store::facts::Facts;
use crate::store::{StorageError, StorageResult};
use amethystate_core::path::StorePath;
use error_stack::{Report, ResultExt};

/// Where `path` is written in `doc`: itself, level by level, in the tree - or
/// one level whose name is the whole of it, in the plane.
///
/// Both are paths, and the second is a path with one level: escaping lives in
/// the joined form and not in the levels, so a level named `widths.left.px`
/// stays one level and reaches the key of that name.
///
/// A plane path is looked for under the spelling this library gives it, and
/// then under the bare name, so a level a person wrote as `.` is reached by the
/// path `["."]` without the file being touched.
pub(super) fn levels<D: TextDocument>(doc: &D, declared: &Declared, path: &StorePath) -> StorePath {
    if path.is_root() || declared.covers(path) {
        return path.clone();
    }

    StorePath::segment(plane_name(doc, path))
}

/// The name in the plane that holds `path`.
fn plane_name<D: TextDocument>(doc: &D, path: &StorePath) -> String {
    let spelled = path.as_str();

    if doc.get(&StorePath::segment(spelled)).is_some() {
        return spelled.to_string();
    }

    match bare_name(path) {
        Some(bare) if doc.get(&StorePath::segment(&bare)).is_some() => bare,
        _ => spelled.to_string(),
    }
}

/// The one level `path` is, written without the escaping - `None` unless the
/// two spellings differ, which is only when the name holds a separator or an
/// escape.
fn bare_name(path: &StorePath) -> Option<String> {
    let name = path.name().filter(|_| path.len() == 1)?;

    (name != path.as_str() && StorePath::parse_joined(&name).is_err()).then(|| name.into_owned())
}

/// The path a name at the root stands for, and which of the two it is.
///
/// A tree's root is one segment that a declaration reaches. Everything else at
/// that level is a whole key, and a name holding a separator can only be one -
/// a tree never gives a level a name with a separator in it, because the levels
/// are where the separators went.
///
/// `key` arrives as a scan hands it over: one level, whose name is what the
/// file calls it. That name is read as the path it spells - or, spelling none,
/// stands as the one level it is. Every name reaches one or the other, because
/// the only name that could reach neither is the empty one, and
/// [`generic_scan`](super::document::generic_scan) drops that before it gets
/// here.
pub(super) fn at_root(declared: &Declared, key: &StorePath) -> StorageResult<(StorePath, Root)> {
    let name = key
        .name()
        .ok_or_else(|| Report::new(StorageError::Path))
        .attach_key(key)
        .attach("a document's root handed back a level with no name")?;

    let path = match StorePath::parse_joined(&name) {
        Ok(path) => path,
        Err(_) => key.clone(),
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
