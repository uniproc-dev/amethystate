use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Control: `ReactiveMap::clear` emits a prefix delete the map recognises.
#[backends(all)]
fn clearing_a_map_notifies_its_subscribers(backend: Backend) {
    let path = TempPath::new("map_clear_notify");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let map = store.kv().map::<String, u32>("columns").unwrap();
    map.insert("cpu".into(), &1).unwrap();

    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let _sub = map.subscribe_any(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });

    map.clear().unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

/// Deleting the map's own prefix through the store removes every entry, but
/// the event carries the path without the trailing separator, which the map's
/// subscription compares against the dotted form - so nothing is notified.
#[backends(all)]
fn deleting_a_maps_prefix_notifies_its_subscribers(backend: Backend) {
    let path = TempPath::new("map_delete_prefix_notify");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let map = store.kv().map::<String, u32>("columns").unwrap();
    map.insert("cpu".into(), &1).unwrap();

    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let _sub = map.subscribe_any(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });

    store.delete_prefix(["columns"]).unwrap();

    assert_eq!(map.len(), 0, "the entries are gone");
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "the map heard about it losing every entry"
    );
}
