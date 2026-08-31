use amethystate::test_utils::unique_store;
use amethystate::uuid;
use amethystate_arena::DefaultArena;
use amethystate_leptos::{use_field, use_map, use_map_subscribe_any, use_map_subscribe_key};
use leptos::prelude::*;
use serial_test::serial;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
struct Probe<T>(Arc<Mutex<Vec<T>>>);
impl<T> Probe<T> {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
    fn push(&self, val: T) {
        self.0.lock().unwrap().push(val);
    }
    fn last(&self) -> Option<T>
    where
        T: Clone,
    {
        self.0.lock().unwrap().last().cloned()
    }
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

impl<T> PartialEq for Probe<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

struct DummyScope;
impl amethystate::StateScope for DummyScope {
    const PATH: amethystate::store::StorePath =
        amethystate::store::StorePath::from_static(&["test"], "test");
    const KEY: &'static str = "test";
}

#[tokio::test(flavor = "current_thread")]
#[serial]
async fn test_use_field_requirements() {
    any_spawner::Executor::init_tokio().ok();

    let store = unique_store("field");
    let arena = DefaultArena::new();

    let field =
        amethystate::store::field_with_path(&store, ["field_1"], 10, uuid::Uuid::new_v4())
            .unwrap();
    let handle = arena.register_field(field);

    let probe = Probe::new();
    let probe_clone = probe.clone();

    let owner = Owner::new();
    owner.set();
    provide_context(arena.clone());

    let (signal, setter) = use_field(handle);

    Effect::new_isomorphic(move |_| {
        probe_clone.push(signal.get());
    });

    leptos::task::tick().await;
    assert_eq!(probe.last(), Some(10));

    setter.set(42);
    leptos::task::tick().await;
    assert_eq!(probe.last(), Some(42));

    let _ = arena.set_field(handle, 100);
    leptos::task::tick().await;
    assert_eq!(probe.last(), Some(100));

    drop(owner);
}

#[tokio::test(flavor = "current_thread")]
#[serial]
async fn test_use_map_requirements() {
    any_spawner::Executor::init_tokio().ok();

    let store = unique_store("map");
    let arena = DefaultArena::new();

    let map = amethystate::store::reactive_map_with_path::<DummyScope, String, String>(
        &store,
        ["map_1"],
        HashMap::new(),
        uuid::Uuid::new_v4(),
    )
    .unwrap();
    let handle = arena.register_map(map);

    let _ = arena.set_map_entry(handle, "key1".to_string(), "val1".to_string());

    let probe = Probe::new();
    let probe_clone = probe.clone();

    let owner = Owner::new();
    owner.set();
    provide_context(arena.clone());

    let map_signal = use_map(handle);

    let _effect = Effect::new_isomorphic(move || {
        probe_clone.push(map_signal.entries.get());
    });

    leptos::task::tick().await;
    tokio::time::sleep(Duration::from_millis(15)).await;

    let initial = probe.last().expect("Entries signal was not loaded");
    assert_eq!(initial.get("key1").unwrap(), "val1");

    map_signal.insert("key2".to_string(), "val2".to_string());
    leptos::task::tick().await;
    assert_eq!(probe.last().unwrap().get("key2").unwrap(), "val2");

    map_signal.remove("key1".to_string());
    leptos::task::tick().await;
    assert!(!probe.last().unwrap().contains_key("key1"));

    map_signal.clear();
    leptos::task::tick().await;
    assert!(probe.last().unwrap().is_empty());

    let _ = arena.set_map_entry(handle, "external".to_string(), "value".to_string());
    leptos::task::tick().await;
    assert_eq!(probe.last().unwrap().get("external").unwrap(), "value");

    drop(owner);
}

#[tokio::test(flavor = "current_thread")]
#[serial]
async fn test_map_sub_requirements() {
    any_spawner::Executor::init_tokio().ok();

    let store = unique_store("sub");
    let arena = DefaultArena::new();

    let map = amethystate::store::reactive_map_with_path::<DummyScope, String, String>(
        &store,
        ["map_2"],
        HashMap::new(),
        uuid::Uuid::new_v4(),
    )
    .unwrap();
    let write_handle = arena.register_map(map);

    let any_changes = Probe::new();
    let any_changes_clone = any_changes.clone();

    let key_changes = Probe::new();
    let key_changes_clone = key_changes.clone();

    let owner = Owner::new();
    owner.set();
    provide_context(arena.clone());

    use_map_subscribe_any(write_handle, move |change| {
        any_changes_clone.push(change.clone());
    });

    use_map_subscribe_key(write_handle, "target".to_string(), move |change| {
        key_changes_clone.push(change.clone());
    });

    let _ = arena.set_map_entry(write_handle, "target".to_string(), "hello".to_string());
    assert_eq!(any_changes.count(), 1);
    assert_eq!(key_changes.count(), 1);

    let _ = arena.set_map_entry(write_handle, "other".to_string(), "world".to_string());
    assert_eq!(any_changes.count(), 2);
    assert_eq!(key_changes.count(), 1);

    drop(owner);

    let _ = arena.set_map_entry(write_handle, "target".to_string(), "dropped".to_string());
    assert_eq!(any_changes.count(), 2);
    assert_eq!(key_changes.count(), 1);
}

#[tokio::test(flavor = "current_thread")]
#[serial]
async fn test_real_component_lifecycle() {
    any_spawner::Executor::init_tokio().ok();

    let store = unique_store("comp");
    let arena = DefaultArena::new();
    let field = amethystate::store::field_with_path(
        &store,
        ["field_1"],
        10,
        uuid::Uuid::new_v4(),
    )
    .unwrap();
    let handle = arena.register_field(field);

    let probe = Probe::new();
    let probe_clone = probe.clone();

    let owner = Owner::new();

    let _view = owner.with(|| {
        provide_context(arena.clone());

        let (signal, _setter) = use_field(handle);

        Effect::new_isomorphic(move || {
            probe_clone.push(signal.get());
        });

        view! { <div>{move || signal.get()}</div> }
    });

    leptos::task::tick().await;
    assert_eq!(probe.last(), Some(10));

    let _ = arena.set_field(handle, 42);

    leptos::task::tick().await;
    assert_eq!(probe.last(), Some(42));

    drop(owner);

    let _ = arena.set_field(handle, 99);
    leptos::task::tick().await;
    assert_eq!(probe.last(), Some(42));
}
