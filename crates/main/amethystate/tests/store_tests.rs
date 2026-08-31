use amethystate::Store;
use amethystate::migration::fields::FieldDescriptor;
use amethystate::migration::set::MigrationSet;
use amethystate::migration::{MigrationError, MigrationPlan};
use amethystate::store::IntoStorageReport;
use amethystate::store::StorageError;
use amethystate::store::SubscriptionKind;
use amethystate::store::config::StoreConfig;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::unique_path;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

const EMPTY_FIELDS: &[FieldDescriptor] = &[];

#[test]
fn test_set_get_immediate() {
    let path = unique_path("immediate");
    let (store, _) = Store::open(StoreConfig::new(path), MigrationSet::default()).unwrap();

    store.set(["user", "name"], &"Alice".to_string()).unwrap();

    let val: Option<String> = store.get(["user", "name"]).unwrap();
    assert_eq!(val, Some("Alice".to_string()));
}

#[test]
fn test_local_reactivity() {
    let path = unique_path("reactivity");
    let (store, _) = Store::open(StoreConfig::new(path), MigrationSet::default()).unwrap();

    let hit = Arc::new(Mutex::new(false));
    let hit_inner = hit.clone();

    store.subscribe(
        SubscriptionKind::ExactPath(StorePath::from_segments(["ui", "theme"])),
        Arc::new(move |_| {
            let mut guard = hit_inner.lock();
            *guard = true;
        }),
    );

    store.set(["ui", "theme"], &"dark".to_string()).unwrap();

    assert!(*hit.lock());
}

#[test]
fn test_delete_flow() {
    let path = unique_path("delete");
    {
        let (store, _) = Store::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();
        store.set(["temp", "key"], &1).unwrap();
        store.save_now().unwrap();
        store.delete(["temp", "key"]).unwrap();
        assert_eq!(store.get::<i32>(["temp", "key"]).unwrap(), None);
        store.save_now().unwrap();
    }

    let (store_reopened, _) =
        Store::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();
    assert_eq!(store_reopened.get::<i32>(["temp", "key"]).unwrap(), None);
}

#[test]
fn bytes_that_are_not_the_type_are_an_error_rather_than_a_default() {
    let path = unique_path("recovery");
    let (store, _) = Store::open(StoreConfig::new(path), MigrationSet::default()).unwrap();
    let garbage = vec![0x00, 0x01, 0x02];

    let err = store.decode::<String>(&garbage).unwrap_err();

    assert_eq!(err.current_context(), &StorageError::Codec);
}

#[test]
fn test_deterministic_closure_and_reopen() {
    let path = unique_path("closure");
    {
        let (store, _) = Store::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();
        store.set(["test", "key"], &"hello".to_string()).unwrap();
    }

    let (store_reopened, _) = Store::open(StoreConfig::new(&path), MigrationSet::default())
        .expect("Database should be available immediately after close");

    let val: Option<String> = store_reopened.get(["test", "key"]).unwrap();
    assert_eq!(val, Some("hello".to_string()));
}

#[test]
fn test_drop_behavior_is_deterministic() {
    let path = unique_path("drop_logic");
    {
        let (store, _) = Store::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();
        store.set(["drop", "test"], &42u32).unwrap();
    }

    let (store_reopened, _) = Store::open(StoreConfig::new(&path), MigrationSet::default())
        .expect("Drop must release file lock deterministically");

    assert_eq!(
        store_reopened.get::<u32>(["drop", "test"]).unwrap(),
        Some(42)
    );
}

#[test]
fn test_close_saves_pending_data() {
    let path = unique_path("save_on_close");
    let mut config = StoreConfig::new(&path);
    config.save_debounce = Duration::from_secs(3600);

    {
        let (store, _) = Store::open(config, MigrationSet::default()).unwrap();
        store.set(["urgent", "data"], &true).unwrap();
    }

    let (store, _) = Store::open(StoreConfig::new(&path), MigrationSet::default()).unwrap();
    assert_eq!(store.get::<bool>(["urgent", "data"]).unwrap(), Some(true));
}

/// A namespace named the way a key on disk spells it.
fn ns(joined: &str) -> StorePath {
    StorePath::parse_joined(joined).expect("a namespace the test wrote itself")
}

#[test]
fn test_is_initialized_false_on_fresh_store() {
    let path = unique_path("init_fresh");
    let (store, _) = Store::open(StoreConfig::new(path), MigrationSet::default()).unwrap();
    assert!(!store.is_initialized(&ns("settings")).unwrap());
}

#[test]
fn test_mark_and_is_initialized() {
    let path = unique_path("init_mark");
    let (store, _) = Store::open(StoreConfig::new(path), MigrationSet::default()).unwrap();
    assert!(!store.is_initialized(&ns("settings")).unwrap());
    store.mark_initialized(&ns("settings")).unwrap();
    assert!(store.is_initialized(&ns("settings")).unwrap());
}

#[test]
fn test_initialized_namespaces_are_independent() {
    let path = unique_path("init_namespaces");
    let (store, _) = Store::open(StoreConfig::new(path), MigrationSet::default()).unwrap();
    store.mark_initialized(&ns("settings")).unwrap();
    assert!(store.is_initialized(&ns("settings")).unwrap());
    assert!(!store.is_initialized(&ns("other")).unwrap());
}

#[test]
fn test_component_atomic_rollback() {
    let path = unique_path("rollback");
    let mut cfg = StoreConfig::new(&path);
    cfg.save_debounce = Duration::from_millis(50);
    {
        let (store, _) = Store::open(cfg, MigrationSet::default()).unwrap();
        store.set(["net", "ip"], &"1.1.1.1".to_string()).unwrap();
        store.save_now().unwrap();
    }

    let mset = MigrationSet::default()
        .add(
            "net",
            MigrationPlan::new().step(1, "ok", |ctx| ctx.set("ip", &"8.8.8.8".to_string())),
            0,
            EMPTY_FIELDS,
            &[],
        )
        .add(
            "ui",
            MigrationPlan::new().step(1, "fail", |_| {
                Err(MigrationError::Custom("crash".into()).into_report())
            }),
            0,
            EMPTY_FIELDS,
            &["net"],
        );

    let (store, report) = Store::open(StoreConfig::new(&path), mset).unwrap();
    assert!(report.has_failures());

    let val: String = store.get(["net", "ip"]).unwrap().unwrap();
    assert_eq!(val, "1.1.1.1");
}
