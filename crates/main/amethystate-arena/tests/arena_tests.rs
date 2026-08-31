#![cfg(not(target_arch = "wasm32"))]
use amethystate::test_utils::unique_store;
use amethystate::{Field, MapChange, ReactiveMap, amethystate};
use amethystate_arena::Arena;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[amethystate(prefix = "arena_test")]
pub struct TestState {
    #[amestate(default = "Alice".to_string())]
    pub username: String,

    #[amestate(default = 8080)]
    pub port: u16,

    #[amestate(default = {})]
    pub sessions: ReactiveMap<String, String>,
}

#[test]
fn test_arena_field() {
    let store = unique_store("field");
    let state = TestState::new_with(&store).unwrap();
    let arena = Arena::new();

    let username_handle = arena.register_field(state.username());
    assert_eq!(arena.get_field(username_handle), "Alice");

    arena
        .set_field(username_handle, "Bob".to_string())
        .unwrap();
    assert_eq!(arena.get_field(username_handle), "Bob");

    let last_val = Arc::new(Mutex::new(String::new()));
    let last_val_clone = last_val.clone();

    let _sub = arena.subscribe_field(username_handle, move |val| {
        *last_val_clone.lock().unwrap() = val.clone();
    });

    arena
        .set_field(username_handle, "Charlie".to_string())
        .unwrap();
    assert_eq!(*last_val.lock().unwrap(), "Charlie");

    let port_handle = arena.register_field(state.port());
    arena.set_field(port_handle, 9090).unwrap();
    assert_eq!(arena.get_field(port_handle), 9090);
}

#[test]
fn test_arena_reactive_map() {
    let store = unique_store("reactive_map");
    let state = TestState::new_with(&store).unwrap();
    let arena = Arena::new();

    let map_handle = arena.register_map(state.sessions());

    arena
        .set_map_entry(map_handle, "user1".to_string(), "token_A".to_string())
        .unwrap();

    let val = arena.get_map_entry(map_handle, &"user1".to_string());
    assert_eq!(val, Some("token_A".to_string()));

    let notified_key = Arc::new(Mutex::new(String::new()));
    let notified_key_clone = notified_key.clone();

    let _sub = arena.subscribe_map_any(map_handle, move |change| {
        if let MapChange::Insert { key, .. } | MapChange::Update { key, .. } = change {
            *notified_key_clone.lock().unwrap() = key.clone();
        }
    });

    arena
        .set_map_entry(map_handle, "user2".to_string(), "token_B".to_string())
        .unwrap();
    assert_eq!(*notified_key.lock().unwrap(), "user2");
}

#[test]
fn test_subscribe_map_key() {
    let store = unique_store("subscribe_map_key");
    let state = TestState::new_with(&store).unwrap();
    let arena = Arena::new();

    let map_handle = arena.register_map(state.sessions());

    let notified_count = Arc::new(Mutex::new(0));
    let notified_count_clone = notified_count.clone();

    let _sub = arena.subscribe_map_key(map_handle, "target_user".to_string(), move |change| {
        if let MapChange::Insert { .. } = change {
            *notified_count_clone.lock().unwrap() += 1;
        }
    });

    arena
        .set_map_entry(map_handle, "other_user".to_string(), "token_A".to_string())
        .unwrap();
    assert_eq!(*notified_count.lock().unwrap(), 0);

    arena
        .set_map_entry(map_handle, "target_user".to_string(), "token_B".to_string())
        .unwrap();
    assert_eq!(*notified_count.lock().unwrap(), 1);
}

#[test]
fn test_arena_cleanup_drops_fields_and_unsubscribes() {
    let store = unique_store("cleanup-test");
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();

    let field: Field<String> = amethystate::store::field_with_path(
        &store,
        ["test", "field"],
        "initial_value".to_string(),
        uuid::Uuid::new_v4(),
    )
    .expect("Failed to initialize field");

    let arena = Arena::new();
    let handle = arena.register_field(field);

    let _sub = arena.subscribe_field(handle, move |_val| {
        calls_clone.fetch_add(1, Ordering::SeqCst);
    });

    arena.set_field(handle, "hello".to_string()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    store.set(["test", "field"], &"world").unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    drop(arena);

    store.set(["test", "field"], &"goodbye").unwrap();

    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "Subscription should be canceled after dropping the arena"
    );
}

#[test]
fn test_arena_remove_map_entry() {
    let store = unique_store("remove_map_entry");
    let state = TestState::new_with(&store).unwrap();
    let arena = Arena::new();

    let map_handle = arena.register_map(state.sessions());

    arena
        .set_map_entry(map_handle, "user1".to_string(), "token_A".to_string())
        .unwrap();
    assert_eq!(
        arena.get_map_entry(map_handle, &"user1".to_string()),
        Some("token_A".to_string())
    );

    let removed = arena
        .remove_map_entry(map_handle, &"user1".to_string())
        .unwrap();
    assert_eq!(removed, Some("token_A".to_string()));
    assert_eq!(arena.get_map_entry(map_handle, &"user1".to_string()), None);

    let removed_again = arena
        .remove_map_entry(map_handle, &"user1".to_string())
        .unwrap();
    assert_eq!(removed_again, None);
}

#[test]
fn test_arena_clear_map() {
    let store = unique_store("clear_map");
    let state = TestState::new_with(&store).unwrap();
    let arena = Arena::new();

    let map_handle = arena.register_map(state.sessions());

    arena
        .set_map_entry(map_handle, "user1".to_string(), "token_A".to_string())
        .unwrap();
    arena
        .set_map_entry(map_handle, "user2".to_string(), "token_B".to_string())
        .unwrap();
    arena
        .set_map_entry(map_handle, "user3".to_string(), "token_C".to_string())
        .unwrap();

    assert_eq!(arena.get_map_entries(map_handle).len(), 3);

    arena.clear_map(map_handle).unwrap();

    assert_eq!(arena.get_map_entries(map_handle).len(), 0);
    assert_eq!(arena.get_map_entry(map_handle, &"user1".to_string()), None);
}

#[test]
fn test_arena_remove_map_entry_fires_subscription() {
    let store = unique_store("remove_fires_sub");
    let state = TestState::new_with(&store).unwrap();
    let arena = Arena::new();

    let map_handle = arena.register_map(state.sessions());
    arena
        .set_map_entry(map_handle, "user1".to_string(), "token_A".to_string())
        .unwrap();

    let removed_keys = Arc::new(Mutex::new(Vec::new()));
    let removed_keys_clone = removed_keys.clone();

    let _sub = arena.subscribe_map_any(map_handle, move |change| {
        if let MapChange::Remove { key, .. } = change {
            removed_keys_clone.lock().unwrap().push(key.clone());
        }
    });

    arena
        .remove_map_entry(map_handle, &"user1".to_string())
        .unwrap();
    assert_eq!(*removed_keys.lock().unwrap(), vec!["user1".to_string()]);
}

#[test]
fn test_arena_clear_map_fires_subscription() {
    let store = unique_store("clear_fires_sub");
    let state = TestState::new_with(&store).unwrap();
    let arena = Arena::new();

    let map_handle = arena.register_map(state.sessions());
    arena
        .set_map_entry(map_handle, "user1".to_string(), "token_A".to_string())
        .unwrap();
    arena
        .set_map_entry(map_handle, "user2".to_string(), "token_B".to_string())
        .unwrap();

    let clear_count = Arc::new(AtomicUsize::new(0));
    let clear_count_clone = clear_count.clone();

    let _sub = arena.subscribe_map_any(map_handle, move |change| {
        if let MapChange::Clear { .. } = change {
            clear_count_clone.fetch_add(1, Ordering::SeqCst);
        }
    });

    arena.clear_map(map_handle).unwrap();
    assert_eq!(clear_count.load(Ordering::SeqCst), 1);
}
