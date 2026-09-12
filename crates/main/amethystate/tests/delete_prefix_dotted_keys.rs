//! A name is allowed to hold the separator, and the escape is what keeps it one
//! name. Everything that reads a key back has to go through the escape rather
//! than through the characters, or a name like `a.exe` turns back into two
//! levels that address nothing.

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::reactive_map_with_path_only;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::collections::HashMap;
use uuid::Uuid;

/// Deleting a subtree has to take the entries whose names hold the separator
/// with it.
#[backends(all)]
fn deleting_a_subtree_takes_the_dotted_names_with_it(backend: Backend) {
    let path = TempPath::new("delete_prefix_dotted");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let map = reactive_map_with_path_only::<String, u32>(
        &store,
        ["dotted", "items"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap();

    map.insert("a.exe".to_string(), &1).unwrap();
    map.insert("plain".to_string(), &2).unwrap();
    assert_eq!(map.len(), 2, "both entries were written");

    drop(map);
    store.delete_prefix(["dotted"]).unwrap();

    let reopened = reactive_map_with_path_only::<String, u32>(
        &store,
        ["dotted", "items"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap();

    assert_eq!(reopened.get("plain"), None, "plain");
    assert_eq!(reopened.get("a.exe"), None, "a.exe");
}

/// A scan of a prefix lists the value stored at the prefix itself, and a name
/// ending in the separator is a name like any other. Asking the joined form
/// whether it ends in a dot cannot tell `cfg.b\.` - one level called `b.` -
/// from a path with an empty level after it, and drops the value.
#[backends(all)]
fn a_scan_lists_the_value_at_a_prefix_whose_name_ends_in_the_separator(backend: Backend) {
    let path = TempPath::new("scan_dotted_prefix");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["cfg", "b."], &7u32).unwrap();

    assert_eq!(
        store.scan_keys(["cfg", "b."]).unwrap(),
        vec![StorePath::from_segments(["cfg", "b."])],
        "the value at the scanned prefix is missing from its own scan"
    );
    assert_eq!(store.get::<u32>(["cfg", "b."]).unwrap(), Some(7));
}
