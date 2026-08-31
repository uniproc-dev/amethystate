//! A path with nothing in it addresses everything.
//!
//! `StorePath::root()` is public and writing to it replaces the whole
//! document, which is at least something a person had to ask for by name. This
//! is about arriving there without asking: a list of segments computed at run
//! time can come out empty, and an empty list is the root.

#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::StoreBuilder;
use amethystate::store::field_with_path;
use amethystate::uuid::Uuid;
use amethystate_core::path::{StorePath, StorePathError};
use amethystate_core::test_utils::TempPath;

mod common;
use common::text_backend;

/// One empty segment is refused, and so is a list of none.
///
/// `try_from_segments` used to walk a list of no segments, find nothing to
/// object to, and return the root - so a path computed at run time that
/// filtered down to nothing addressed the whole store, and there was no error
/// for it to return because `StorePathError` had none.
#[test]
fn an_empty_segment_is_refused_and_so_is_an_empty_list() {
    assert_eq!(
        StorePath::try_from_segments(["ui", ""]).unwrap_err(),
        StorePathError::EmptySegment { at: 1 },
        "an empty segment is refused by name and by position, which is the \
         behaviour this contrasts with"
    );

    let nothing: Vec<String> = Vec::new();
    assert_eq!(
        StorePath::try_from_segments(&nothing).unwrap_err(),
        StorePathError::EmptyPath,
        "a list of no segments was accepted, and what it names is everything"
    );

    assert!(
        StorePath::root().is_root(),
        "the root is still reachable, by name, which is the point of refusing \
         the other way in"
    );
}

/// What that costs when the empty list reaches a write.
///
/// Nothing here names the root. The segments are computed, the filter happens
/// to remove all of them, and the write that follows returns success.
///
/// A scalar at the root was refused by the guard that stops a scalar landing on
/// a branch, which is why the shape that got through was the ordinary one: a
/// struct or a map, written at a path that came out empty. That guard was never
/// meant for this and covered it by accident.
#[test]
fn a_path_that_filtered_down_to_nothing_does_not_replace_the_store() {
    let path = TempPath::new("empty_path_write");

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();

    let kept = field_with_path::<u32>(&store, ["ui", "width"], 1280, Uuid::new_v4()).unwrap();
    kept.set(1920).unwrap();
    store.save_now().unwrap();

    let wanted = ["", ""];
    let computed: Vec<&str> = wanted.iter().copied().filter(|s| !s.is_empty()).collect();

    let mut value = std::collections::HashMap::new();
    value.insert("theme".to_string(), "dark".to_string());
    let wrote = store.set(computed, &value);

    store.save_now().unwrap();

    assert!(
        wrote.is_err(),
        "a write at a path that came out empty was accepted; the file is now {}",
        std::fs::read_to_string(path.path()).unwrap_or_default()
    );
    assert_eq!(
        kept.get(),
        1920,
        "a write at a path that came out empty replaced the whole document"
    );
}
