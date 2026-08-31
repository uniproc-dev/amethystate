use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{MapChange, ReactiveMap, amethystate};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;
use std::sync::{Arc, Mutex};

#[amethystate(prefix = "ic")]
pub struct Cfg {
    #[amestate(default = 0u64)]
    pub counter: u64,

    #[amestate(default = {"a": 1u64, "b": 2u64})]
    pub items: ReactiveMap<String, u64>,
}

fn cfg(backend: Backend) -> (amethystate::Store, Cfg) {
    let path = unique_path("interceptors");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();
    (store, cfg)
}

/// A value the interceptor turns down: every level of the recursion tries to
/// write it, so one attempt lands at whatever depth the guard stops at, without
/// this test knowing what that depth is.
const REJECTED: u64 = 999;

/// Past the depth limit the interceptor cannot run, so the write is refused
/// rather than let through. Letting it through means a validator stops being
/// consulted exactly where recursion is deepest, and the value it exists to
/// reject reaches the store.
#[backends(all)]
fn a_rejected_value_never_reaches_the_store_however_deep_the_recursion(backend: Backend) {
    let (store, cfg) = cfg(backend);

    let counter = cfg.counter();
    let nested = counter.clone();

    let _guard = counter.intercept(move |change| {
        if change.new_value >= REJECTED {
            return None;
        }
        let _ = nested.set(REJECTED);
        let _ = nested.set(change.new_value + 1);
        Some(change)
    });

    let reached = Arc::new(Mutex::new(Vec::new()));
    let cap = reached.clone();
    let _sub = counter.subscribe(move |v: &u64| cap.lock().unwrap().push(*v));

    let _ = counter.set(1);

    assert!(
        !reached.lock().unwrap().contains(&REJECTED),
        "a subscriber saw the rejected value: {:?}",
        reached.lock().unwrap()
    );
    assert_ne!(
        store.get::<u64>(["ic", "counter"]).unwrap(),
        Some(REJECTED),
        "the rejected value reached the store"
    );
}

/// A `Clear` is not about any one key, so key interceptors must not see it.
/// Running all of them handed each the same change and accumulated their
/// rewrites, in an order that varied between runs.
#[backends(all)]
fn clear_does_not_run_key_interceptors(backend: Backend) {
    let (_s, cfg) = cfg(backend);
    let items = cfg.items();

    let hits = Arc::new(Mutex::new(Vec::new()));
    let mut guards = Vec::new();
    for key in ["a", "b"] {
        let hits = hits.clone();
        let owned = key.to_string();
        guards.push(items.intercept_key(key.to_string(), move |change| {
            hits.lock().unwrap().push(owned.clone());
            Some(change)
        }));
    }

    items.clear().unwrap();

    assert!(
        hits.lock().unwrap().is_empty(),
        "key interceptors ran for a keyless change: {:?}",
        hits.lock().unwrap()
    );
}

#[backends(all)]
fn key_interceptors_still_run_for_their_own_key(backend: Backend) {
    let (_s, cfg) = cfg(backend);
    let items = cfg.items();

    let hits = Arc::new(Mutex::new(Vec::new()));
    let cap = hits.clone();
    let _guard = items.intercept_key("a".to_string(), move |change| {
        cap.lock().unwrap().push(change.key().cloned());
        Some(change)
    });

    items.update("a", &10).unwrap();
    items.update("b", &20).unwrap();

    assert_eq!(*hits.lock().unwrap(), vec![Some("a".to_string())]);
}

/// A global interceptor is about the map, so it does see a `Clear`.
#[backends(all)]
fn clear_still_reaches_a_global_interceptor(backend: Backend) {
    let (_s, cfg) = cfg(backend);
    let items = cfg.items();

    let seen = Arc::new(Mutex::new(0usize));
    let cap = seen.clone();
    let _guard = items.intercept(move |change| {
        if matches!(change, MapChange::Clear { .. }) {
            *cap.lock().unwrap() += 1;
        }
        Some(change)
    });

    items.clear().unwrap();

    assert_eq!(*seen.lock().unwrap(), 1);
}
