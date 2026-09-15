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

#[test]
fn a_list_of_no_segments_is_refused_where_a_list_of_empty_ones_is_not() {
    let held = StorePath::try_from_segments(["ui", ""]).unwrap();
    assert_eq!(held.len(), 2, "an empty name is a name, and names a level");

    let nothing: Vec<String> = Vec::new();
    assert_eq!(
        StorePath::try_from_segments(&nothing).unwrap_err(),
        StorePathError::EmptyPath,
        "a list of no levels names everything, and nobody asked for that"
    );

    assert!(
        StorePath::root().is_root(),
        "the root is reachable by name, which is what makes refusing the other \
         way in affordable"
    );
}

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
