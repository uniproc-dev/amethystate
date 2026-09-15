use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, Store, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "ord")]
pub struct Cfg {
    #[amestate(default = {})]
    pub items: ReactiveMap<String, u64>,
}

const SEEDED: [&str; 5] = ["zulu", "alpha", "mike", "bravo", "delta"];
const SORTED: [&str; 5] = ["alpha", "bravo", "delta", "mike", "zulu"];

fn seeded(backend: Backend, path: &std::path::Path) -> (Store, Cfg) {
    let store = StoreBuilder::new(path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    for k in SEEDED {
        cfg.items().insert(k.to_string(), &1).unwrap();
    }

    (store, cfg)
}

/// The keys a map holds after it is built again from the store, which is the
/// walk that goes through `scan_prefix` and the write buffer.
fn reloaded(store: &Store, held: Cfg) -> (Cfg, Vec<String>) {
    drop(held);
    let again = Cfg::new_with(store).unwrap();
    let keys = again.items().keys().collect();
    (again, keys)
}

#[backends(all)]
fn a_map_built_from_the_store_comes_back_sorted(backend: Backend) {
    let path = TempPath::new("order_sorted");
    let (store, cfg) = seeded(backend, &path);
    store.save_now().unwrap();

    let (_cfg, keys) = reloaded(&store, cfg);

    assert_eq!(keys, SORTED);
}

/// `scan_prefix` merges committed keys with the not-yet-flushed write buffer,
/// and that buffer is a hash map. Unsorted, its iteration order leaked out: keys
/// came back in one order before a flush and another after it, so a view
/// listing them reordered itself mid-session, differently on every run.
///
/// Reading the live map cannot see this - it holds a sorted tree of its own and
/// a flush does not touch it. The map has to be built again on each side of the
/// flush, which is what puts the buffer merge between the store and the answer.
#[backends(all)]
fn entry_order_survives_a_flush(backend: Backend) {
    let path = TempPath::new("order_flush");
    let (store, cfg) = seeded(backend, &path);

    let (cfg, before) = reloaded(&store, cfg);
    store.save_now().unwrap();
    let (_cfg, after) = reloaded(&store, cfg);

    assert_eq!(before, SORTED, "before the flush, from the write buffer");
    assert_eq!(after, before, "the flush moved the keys and reordered them");
}

#[backends(all)]
fn what_a_map_holds_is_what_the_store_holds(backend: Backend) {
    let path = TempPath::new("order_agree");
    let (store, cfg) = seeded(backend, &path);

    let held: Vec<String> = cfg.items().keys().collect();
    let stored: Vec<String> = store
        .scan_keys(["ord", "items"])
        .unwrap()
        .into_iter()
        .filter_map(|key| key.name().map(|name| name.as_str().to_string()))
        .collect();

    assert_eq!(held, SORTED);
    assert_eq!(stored, held);
}

#[backends(all)]
fn a_rebuilt_map_sees_a_write_that_was_never_flushed(backend: Backend) {
    let path = TempPath::new("order_unflushed");
    let (store, cfg) = seeded(backend, &path);
    store.save_now().unwrap();

    cfg.items().insert("zzz".to_string(), &1).unwrap();
    let (_cfg, keys) = reloaded(&store, cfg);

    assert_eq!(keys, ["alpha", "bravo", "delta", "mike", "zulu", "zzz"]);
}

#[backends(all)]
fn a_rebuilt_map_forgets_an_entry_removed_since_the_flush(backend: Backend) {
    let path = TempPath::new("order_removed");
    let (store, cfg) = seeded(backend, &path);
    store.save_now().unwrap();

    cfg.items().remove("mike").unwrap();
    let (_cfg, keys) = reloaded(&store, cfg);

    assert_eq!(keys, ["alpha", "bravo", "delta", "zulu"]);
}
