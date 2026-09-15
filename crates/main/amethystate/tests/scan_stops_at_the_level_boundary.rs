//! A scan of `ui` answers for `ui`, and for nothing that merely starts with it.
//!
//! `backend_conformance` claims this already, and cannot see it: those
//! properties open with a sixty-second debounce and never flush, so on the
//! engines that buffer, the scan reads no committed rows at all and only
//! exercises the buffer - which filters correctly. The engine's own range query
//! is never asked.
//!
//! So these flush first, and they name their sibling rather than generating it,
//! because the character that breaks it is a property of the key encoding and
//! not something a run should have to be lucky to find.
//!
//! Every engine, named explicitly. A store that is wrong here loses data
//! through `delete_prefix`, which is built on the same scan.

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

mod common;
use common::once_per_engine;

/// Names that are one level, are not `ui`, and begin with `ui`.
///
/// `key_range` bounds a scan of `ui` below by `"ui"` and above by
/// `"ui." + U+10FFFF`, so a name whose third character sorts below `.` (U+002E)
/// falls inside that range while belonging to nobody. The whole class is
/// U+0000 through U+002D; these are the printable ones a person might type.
const SIBLINGS: &[&str] = &[
    "ui x", "ui!x", "ui\"x", "ui#x", "ui%x", "ui'x", "ui-x", "ui,x",
];

/// A name above the range's upper bound, which is the same defect from the
/// other end: a scan of `ui` cannot reach it, so it survives a delete of its
/// own subtree.
const ABOVE_THE_BOUND: &str = "\u{10FFFF}z";

fn seeded(label: &str, backend: Backend, sibling: &str) -> (TempPath, amethystate::Store) {
    let path = TempPath::new(label);
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["ui", "width"], &1280u32).unwrap();
    store.set([sibling], &7u32).unwrap();

    // The point of the file: without this the scan below reads the write
    // buffer, which filters correctly on every engine, and proves nothing about
    // the query the engine runs against what it has committed.
    store.save_now().unwrap();

    (path, store)
}

fn scan_lists_only_the_subtree(backend: Backend, label: &str) {
    for sibling in SIBLINGS {
        let (_path, store) = seeded(label, backend, sibling);

        let listed = store.scan_keys(["ui"]).unwrap();
        assert_eq!(
            listed,
            vec![StorePath::from_segments(["ui", "width"])],
            "a scan of `ui` claimed `{sibling}` was under it, and `delete_prefix` \
             is built on this scan"
        );
    }
}

fn delete_prefix_takes_only_the_subtree(backend: Backend, label: &str) {
    for sibling in SIBLINGS {
        let (_path, store) = seeded(label, backend, sibling);

        store.delete_prefix(["ui"]).unwrap();
        store.save_now().unwrap();

        assert_eq!(
            store.get::<u32>([*sibling]).unwrap(),
            Some(7),
            "deleting the `ui` subtree destroyed `{sibling}`, which was never in it"
        );
    }
}

fn a_child_above_the_bound_is_still_a_child(backend: Backend, label: &str) {
    let path = TempPath::new(label);
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["ui", ABOVE_THE_BOUND], &7u32).unwrap();
    store.save_now().unwrap();

    let listed = store.scan_keys(["ui"]).unwrap();
    assert_eq!(
        listed,
        vec![StorePath::from_segments(["ui", ABOVE_THE_BOUND])],
        "a scan of `ui` did not reach its own child, so nothing under that name \
         can be deleted, migrated or inspected"
    );
}

once_per_engine! {
    #[test]
    fn a_scan_lists_only_what_is_under_the_prefix() {
        scan_lists_only_the_subtree(BACKEND, &format!("scan_{ENGINE}"));
    }

    #[test]
    fn deleting_a_prefix_takes_only_its_subtree() {
        delete_prefix_takes_only_the_subtree(BACKEND, &format!("del_{ENGINE}"));
    }

    #[test]
    fn a_child_above_the_range_bound_is_still_scanned() {
        a_child_above_the_bound_is_still_a_child(BACKEND, &format!("bound_{ENGINE}"));
    }
}
