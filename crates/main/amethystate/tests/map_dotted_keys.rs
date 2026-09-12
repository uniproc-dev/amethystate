use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::reactive_map_with_path_only;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::collections::HashMap;
use uuid::Uuid;

#[backends(all)]
fn keys_containing_the_separator_stay_separate_entries(backend: Backend) {
    let path = TempPath::new("map_dotted");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let map = reactive_map_with_path_only::<String, u32>(
        &store,
        ["dotted", "items"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap();

    map.insert("a.exe".to_string(), &1).unwrap();
    map.insert("a.dll".to_string(), &2).unwrap();
    map.insert("b.exe".to_string(), &3).unwrap();

    assert_eq!(map.get("a.exe"), Some(1), "get a.exe");
    assert_eq!(map.get("a.dll"), Some(2), "get a.dll");
    assert_eq!(map.get("b.exe"), Some(3), "get b.exe");
    assert_eq!(map.keys().count(), 3, "keys");
    assert_eq!(map.entries().count(), 3, "entries");
    assert_eq!(map.len(), 3, "len");

    drop(map);
    let reopened = reactive_map_with_path_only::<String, u32>(
        &store,
        ["dotted", "items"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap();

    assert_eq!(reopened.len(), 3, "len after a reopen");
    assert_eq!(reopened.get("a.exe"), Some(1), "get after a reopen");
}

#[backends(all)]
fn a_key_that_is_a_prefix_of_another_keeps_its_own_value(backend: Backend) {
    let path = TempPath::new("map_dotted_collide");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let map = reactive_map_with_path_only::<String, u32>(
        &store,
        ["collide", "items"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap();

    map.insert("a".to_string(), &1).unwrap();
    map.insert("a.b".to_string(), &2).unwrap();

    assert_eq!(map.get("a"), Some(1), "leaf survived");
    assert_eq!(map.get("a.b"), Some(2), "branch");

    drop(map);
    let reopened = reactive_map_with_path_only::<String, u32>(
        &store,
        ["collide", "items"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap();

    assert_eq!(reopened.get("a"), Some(1), "leaf survived a reopen");
    assert_eq!(reopened.get("a.b"), Some(2), "branch survived a reopen");
}
