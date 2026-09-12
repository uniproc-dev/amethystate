use amethystate::store::StoreBackend;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::collections::HashMap;

/// The same three questions the text engines answer wrongly, asked of whichever
/// engine the build enabled. A flat engine stores the level's key `\.` whole and
/// has nothing that maps it to the root, so this is where the engines diverge.
#[test]
fn a_level_named_dot_is_an_ordinary_level() {
    let path = TempPath::new("tamper_dot_contrast");
    let store = StoreBuilder::new(path.path()).build().unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();

    let dot = StorePath::segment(".");
    assert_eq!(
        StoreBackend::get_raw(&store, &dot).unwrap(),
        None,
        "nothing was written at `.`"
    );

    let mut payload: HashMap<String, u32> = HashMap::new();
    payload.insert("zzz".to_string(), 1);
    store.set(["."], &payload).unwrap();

    assert_eq!(
        store.get::<u32>(["cfg", "width"]).unwrap(),
        Some(1280),
        "a write at `.` destroyed an unrelated value"
    );

    StoreBackend::delete(&store, &dot).unwrap();
    assert_eq!(
        StoreBackend::get_raw(&store, &dot).unwrap(),
        None,
        "`.` is still there after deleting it"
    );
}

/// A scan over a prefix includes the value stored at the prefix itself, on
/// every engine. Recorded here because a caller that treats a scan result as
/// "the entries of a map" sees the map's own node among them, and the text
/// engines reach it by a different route than the flat ones.
#[test]
fn a_value_at_the_prefix_is_listed_by_a_scan_over_it() {
    let path = TempPath::new("tamper_leaf_prefix");
    let store = StoreBuilder::new(path.path()).build().unwrap();

    store.set(["items"], &7u32).unwrap();

    let keys = StoreBackend::scan_keys(&store, &StorePath::segment("items")).unwrap();
    assert_eq!(keys, vec![StorePath::segment("items")]);
}
