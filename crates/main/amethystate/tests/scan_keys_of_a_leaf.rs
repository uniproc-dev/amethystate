//! What a scan of a path with nothing under it answers.
//!
//! It answers with that path. A scan names the subtree rooted at the prefix,
//! the root included, so a leaf is its own only member - and every engine
//! agrees, which is what conformance property 7 is for: `is_under` admits
//! `key == prefix` on the flat engines, and the text engines' recursion does
//! the same.
//!
//! Worth a file of its own because it reads like a defect and is not, and
//! because it is a genuine trap: a walk that recurses into whatever a scan
//! returned never gets closer to the bottom - the leaf hands back the path it
//! was given. Anything walking the key space has to stop when a scan answers
//! with the prefix it was asked about.

#![cfg(any(
    feature = "redb",
    feature = "sqlite",
    feature = "json",
    feature = "toml",
    feature = "ron"
))]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[backends(all)]
fn a_leaf_answers_with_itself(backend: Backend) {
    let path = TempPath::new("leaf");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["leafy", "value"], &7u32).unwrap();

    assert_eq!(
        store.scan_keys(["leafy"]).unwrap(),
        vec![StorePath::from_segments(["leafy", "value"])],
        "the level above the leaf names it, which is the control for the case below"
    );

    assert_eq!(
        store.scan_keys(["leafy", "value"]).unwrap(),
        vec![StorePath::from_segments(["leafy", "value"])],
        "a scan is the subtree including its root, so a leaf is its own only member"
    );

    // The half that makes it a contract rather than an accident: the two scans
    // must say the same thing, which is what conformance property 7 asserts
    // across generated inputs.
    let entries: Vec<StorePath> = store
        .scan_prefix(["leafy", "value"])
        .unwrap()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(
        entries,
        store.scan_keys(["leafy", "value"]).unwrap(),
        "`scan_keys` and `scan_prefix` disagreed about a leaf"
    );
}

#[backends(all)]
fn a_path_that_is_not_there_has_no_members(backend: Backend) {
    let path = TempPath::new("absent");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    assert!(
        store.scan_keys(["nothing", "here"]).unwrap().is_empty(),
        "a path nobody wrote has no subtree, not even itself"
    );
}
