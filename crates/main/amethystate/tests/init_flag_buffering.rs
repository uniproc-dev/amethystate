use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;

/// A namespace named the way a key on disk spells it.
fn ns(joined: &str) -> StorePath {
    StorePath::parse_joined(joined).expect("a namespace the test wrote itself")
}

#[backends(all)]
fn a_mark_is_visible_before_it_reaches_disk(backend: Backend) {
    let store = StoreBuilder::new(unique_path("readback"))
        .backend(backend)
        .build()
        .unwrap();

    assert!(!store.is_initialized(&ns("settings")).unwrap());
    store.mark_initialized(&ns("settings")).unwrap();

    assert!(
        store.is_initialized(&ns("settings")).unwrap(),
        "the buffer answers, as it does for get"
    );
}

#[backends(all)]
fn a_mark_survives_a_flush_and_reopen(backend: Backend) {
    let path = unique_path("persist");

    {
        let store = StoreBuilder::new(path.clone())
            .backend(backend)
            .build()
            .unwrap();
        store.mark_initialized(&ns("settings")).unwrap();
        store.save_now().unwrap();
    }

    let store = StoreBuilder::new(path).backend(backend).build().unwrap();
    assert!(store.is_initialized(&ns("settings")).unwrap());
}

/// A mark and the values beside it come back together after a prefix flush.
///
/// Which of the two wrote them is not settled here: the store is dropped on
/// the way out, and a drop flushes whatever is still buffered, so this holds
/// whether the flush carried the mark or the drop did. Telling those apart
/// needs a process that dies without running destructors, as
/// `durability_crash.rs` does.
#[backends(all)]
fn a_prefix_flush_carries_the_mark_for_that_namespace(backend: Backend) {
    let path = unique_path("prefix");

    {
        let store = StoreBuilder::new(path.clone())
            .backend(backend)
            .build()
            .unwrap();
        store.set(["settings", "port"], &8080u16).unwrap();
        store.mark_initialized(&ns("settings")).unwrap();

        store.flush_prefix(["settings"]).unwrap();
    }

    let store = StoreBuilder::new(path).backend(backend).build().unwrap();
    assert!(
        store.is_initialized(&ns("settings")).unwrap(),
        "the mark is selected by the same prefix rule as its values"
    );
    assert_eq!(store.get::<u16>(["settings", "port"]).unwrap(), Some(8080));
}

#[backends(all)]
fn a_mark_does_not_show_up_as_data(backend: Backend) {
    let store = StoreBuilder::new(unique_path("notdata"))
        .backend(backend)
        .build()
        .unwrap();

    store.mark_initialized(&ns("settings")).unwrap();
    store.set(["settings", "port"], &1u16).unwrap();

    let found = store.scan_prefix(["settings"]).unwrap();
    let keys: Vec<&str> = found.iter().map(|(k, _)| k.as_str()).collect();

    assert_eq!(
        keys,
        vec!["settings.port"],
        "the flag shares the buffer but is not a value"
    );
}
