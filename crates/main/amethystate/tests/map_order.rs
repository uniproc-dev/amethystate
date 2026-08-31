use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;

#[amethystate(prefix = "ord")]
pub struct Cfg {
    #[amestate(default = {})]
    pub items: ReactiveMap<String, u64>,
}

fn seeded(backend: Backend, path: &std::path::Path) -> (amethystate::Store, Cfg) {
    let store = StoreBuilder::new(path)
        .backend(backend)
        .build()
        .unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    for k in ["zulu", "alpha", "mike", "bravo", "delta"] {
        cfg.items().insert(k.to_string(), &1).unwrap();
    }

    (store, cfg)
}

#[backends(all)]
fn entries_are_sorted_by_key(backend: Backend) {
    let path = unique_path("order_sorted");
    let (_store, cfg) = seeded(backend, &path);

    assert_eq!(
        cfg.items().keys().collect::<Vec<_>>(),
        ["alpha", "bravo", "delta", "mike", "zulu"]
    );
}

/// `scan_prefix` merges committed keys with the not-yet-flushed write buffer,
/// and that buffer is a hash map. Unsorted, its iteration order leaked out: keys
/// came back in one order before a flush and another after it, so a view
/// listing them reordered itself mid-session, differently on every run.
#[backends(all)]
fn entry_order_survives_a_flush(backend: Backend) {
    let path = unique_path("order_flush");
    let (store, cfg) = seeded(backend, &path);

    let before: Vec<String> = cfg.items().keys().collect();
    store.save_now().unwrap();
    let after: Vec<String> = cfg.items().keys().collect();

    assert_eq!(before, after);
}

#[backends(all)]
fn entries_and_keys_agree(backend: Backend) {
    let path = unique_path("order_agree");
    let (_store, cfg) = seeded(backend, &path);

    let from_entries: Vec<String> = cfg.items().entries().map(|(k, _)| k).collect();

    assert_eq!(from_entries, cfg.items().keys().collect::<Vec<_>>());
}

/// `keys()` must agree with `entries()` on every backend, including for keys
/// that are still only in the write buffer.
#[backends(all)]
fn keys_sees_unflushed_writes(backend: Backend) {
    let path = unique_path("order_unflushed");
    let (_store, cfg) = seeded(backend, &path);

    cfg.items().insert("zzz".to_string(), &1).unwrap();

    let keys: Vec<String> = cfg.items().keys().collect();
    assert!(keys.contains(&"zzz".to_string()));

    let from_entries: Vec<String> = cfg.items().entries().map(|(k, _)| k).collect();
    assert_eq!(keys, from_entries);
}

#[backends(all)]
fn keys_forgets_a_removed_entry(backend: Backend) {
    let path = unique_path("order_removed");
    let (store, cfg) = seeded(backend, &path);

    store.save_now().unwrap();
    cfg.items().remove("mike").unwrap();

    assert_eq!(
        cfg.items().keys().collect::<Vec<_>>(),
        ["alpha", "bravo", "delta", "zulu"]
    );
}
