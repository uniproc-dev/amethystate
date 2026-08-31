use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{MapChange, ReactiveMap, amethystate};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;
use std::sync::{Arc, Mutex};

#[amethystate(prefix = "old")]
pub struct Cfg {
    #[amestate(default = {})]
    pub items: ReactiveMap<String, u64>,
}

/// `old_value` used to come from the write buffer alone, so once a flush had
/// emptied it the next write reported no old value - and the map turned that
/// into the type's default. Comparing old against new in a subscriber worked on
/// the text backends and quietly did not on redb and sqlite.
#[backends(all)]
fn an_update_reports_the_value_that_was_there_before(backend: Backend) {
    let path = unique_path("old_value");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    cfg.items().insert("k".into(), &5).unwrap();
    store.save_now().unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let cap = seen.clone();
    let _sub = cfg
        .items()
        .subscribe_any(move |change: &MapChange<String, u64>| {
            if let MapChange::Update {
                old_value,
                new_value,
                ..
            } = change
            {
                cap.lock().unwrap().push((*old_value, *new_value));
            }
        });

    cfg.items().update("k", &7).unwrap();

    assert_eq!(
        *seen.lock().unwrap(),
        vec![(5, 7)],
        "the flushed value is still the old one"
    );
}

#[backends(all)]
fn an_unflushed_write_is_the_old_value_for_the_next_one(backend: Backend) {
    let path = unique_path("old_value_buffered");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    cfg.items().insert("k".into(), &1).unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let cap = seen.clone();
    let _sub = cfg
        .items()
        .subscribe_any(move |change: &MapChange<String, u64>| {
            if let MapChange::Update { old_value, .. } = change {
                cap.lock().unwrap().push(*old_value);
            }
        });

    cfg.items().update("k", &2).unwrap();

    assert_eq!(
        *seen.lock().unwrap(),
        vec![1],
        "the buffer holds the newer value and wins over the database"
    );
}

#[backends(all)]
fn a_removal_reports_the_flushed_value(backend: Backend) {
    let path = unique_path("old_value_remove");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    cfg.items().insert("k".into(), &42).unwrap();
    store.save_now().unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let cap = seen.clone();
    let _sub = cfg
        .items()
        .subscribe_any(move |change: &MapChange<String, u64>| {
            if let MapChange::Remove { old_value, .. } = change {
                cap.lock().unwrap().push(*old_value);
            }
        });

    cfg.items().remove("k").unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![42]);
}
