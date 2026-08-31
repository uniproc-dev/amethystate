use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

/// A prefix is a run of whole levels, so a sibling whose name merely starts
/// with the same characters is a different subtree.
#[backends(all)]
fn scanning_a_prefix_stops_at_a_segment_boundary(backend: Backend) {
    let path = TempPath::new("prefix_scan");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let kv = store.kv();

    kv.namespace("ui").set("width", &1280u32).unwrap();
    kv.namespace("uix").set("width", &640u32).unwrap();

    let keys = kv.namespace("ui").keys().unwrap();
    assert_eq!(
        keys.iter().map(StorePath::as_str).collect::<Vec<_>>(),
        ["ui.width"],
        "uix is a different subtree"
    );
}

/// The same boundary reached through a delete, which is where getting it wrong
/// costs data rather than an extra row.
#[backends(all)]
fn deleting_a_prefix_leaves_a_sibling_with_a_shared_leading_string(backend: Backend) {
    let path = TempPath::new("prefix_delete");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let kv = store.kv();

    kv.namespace("ui").set("width", &1280u32).unwrap();
    kv.namespace("uix").set("width", &640u32).unwrap();

    store.delete_prefix(["ui"]).unwrap();

    assert_eq!(kv.namespace("ui").get::<u32>("width").unwrap(), None);
    assert_eq!(
        kv.namespace("uix").get::<u32>("width").unwrap(),
        Some(640),
        "a sibling subtree survives"
    );
}

/// A name may hold anything, including whatever a pattern language would treat
/// as special - `cfg[a]` is a name, not a character class. The engines scan by
/// comparison rather than by pattern, so there is nothing to escape.
#[backends(all)]
fn a_path_segment_with_glob_metacharacters_scans_as_a_literal(backend: Backend) {
    let path = TempPath::new("prefix_glob");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let kv = store.kv();

    kv.namespace("cfg[a]").set("width", &1u32).unwrap();
    kv.namespace("cfg[a]").set("height", &2u32).unwrap();
    store.flush_prefix(StorePath::root()).unwrap();

    assert_eq!(kv.namespace("cfg[a]").get::<u32>("width").unwrap(), Some(1));

    let mut keys = kv.namespace("cfg[a]").keys().unwrap();
    keys.sort();
    assert_eq!(
        keys.iter().map(StorePath::as_str).collect::<Vec<_>>(),
        ["cfg[a].height", "cfg[a].width"],
        "the bracket is part of the path, not a character class"
    );
}

/// The same over a map, whose `len`, `keys` and `clear` all ride the scan.
#[backends(all)]
fn a_map_at_a_path_with_glob_metacharacters_reports_its_entries(backend: Backend) {
    let path = TempPath::new("map_glob");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let map = store
        .kv()
        .namespace("panel[0]")
        .map::<String, u32>("widths")
        .unwrap();

    map.insert("cpu".into(), &120).unwrap();
    map.insert("mem".into(), &80).unwrap();
    store.flush_prefix(StorePath::root()).unwrap();

    assert_eq!(map.get("cpu"), Some(120));
    assert_eq!(map.len(), 2, "both entries are visible");

    map.clear().unwrap();
    store.flush_prefix(StorePath::root()).unwrap();
    assert_eq!(
        store.get::<u32>(["panel[0]", "widths", "cpu"]).unwrap(),
        None,
        "clear removed the entries"
    );
}

/// Control: the identical map with no metacharacter in its path.
#[backends(all)]
fn a_map_at_a_plain_path_reports_its_entries(backend: Backend) {
    let path = TempPath::new("map_plain");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let map = store
        .kv()
        .namespace("panel0")
        .map::<String, u32>("widths")
        .unwrap();

    map.insert("cpu".into(), &120).unwrap();
    map.insert("mem".into(), &80).unwrap();
    store.flush_prefix(StorePath::root()).unwrap();

    assert_eq!(map.len(), 2);

    map.clear().unwrap();
    store.flush_prefix(StorePath::root()).unwrap();
    assert_eq!(store.get::<u32>(["panel0", "widths", "cpu"]).unwrap(), None);
}
