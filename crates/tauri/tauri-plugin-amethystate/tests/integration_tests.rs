use amethystate::test_utils::unique_store;
use tauri_plugin_amethystate::backend::commands::PluginState;

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn test_tauri_plugin_commands() {
    use tauri::Manager;
    let (_at, store) = unique_store("amethystate_tauri_test_store.redb");

    store.set(["test_root", "value"], &100i32).unwrap();
    store.save_now().unwrap();

    let app = tauri::test::mock_app();

    let plugin_state = PluginState {
        subscriptions: Default::default(),
        store: store.clone(),
    };

    app.manage(plugin_state);

    let plugin_state = app.state::<PluginState>();

    let val = tauri_plugin_amethystate::backend::commands::amethystate_get(
        plugin_state.clone(),
        "test_root.value".to_string(),
    )
    .await;
    assert_eq!(val, Ok(Some(serde_json::json!(100))));

    let set_res = tauri_plugin_amethystate::backend::commands::amethystate_set(
        plugin_state,
        "test_root.value".to_string(),
        serde_json::json!(200),
        None,
    )
    .await;
    assert_eq!(set_res, Ok(()));

    let updated_val: Option<i32> = store.get(["test_root", "value"]).unwrap();
    assert_eq!(updated_val, Some(200));
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn a_path_stays_watched_while_anyone_still_watches_it() {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::{Listener, Manager};
    use tauri_plugin_amethystate::backend::commands::{
        amethystate_subscribe, amethystate_unsubscribe,
    };

    let (_at, store) = unique_store("watched_by_two");
    let app = tauri::test::mock_app();
    app.manage(PluginState {
        subscriptions: Default::default(),
        store: store.clone(),
    });

    let heard = Arc::new(Mutex::new(0usize));
    let sink = heard.clone();
    app.listen_any("amethystate://todos:items", move |_| {
        *sink.lock().unwrap() += 1
    });

    let state = app.state::<PluginState>();
    let watch = || amethystate_subscribe(state.clone(), app.handle().clone(), "todos.items".into());
    watch().await.unwrap();
    watch().await.unwrap();

    let heard_after = |write: u32| {
        store.set(["todos", "items", "3"], &write).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        *heard.lock().unwrap()
    };

    amethystate_unsubscribe(state.clone(), "todos.items".into())
        .await
        .unwrap();
    assert_eq!(heard_after(1), 1, "one watcher is still there");

    amethystate_unsubscribe(state.clone(), "todos.items".into())
        .await
        .unwrap();
    assert_eq!(heard_after(2), 1, "nobody watches any more");
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn an_entry_arrives_under_its_own_name_and_says_who_wrote_it() {
    use amethystate::store::StorePath;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::{Listener, Manager};
    use tauri_plugin_amethystate::backend::commands::amethystate_subscribe;

    let (_at, store) = unique_store("entry_named");
    let app = tauri::test::mock_app();
    app.manage(PluginState {
        subscriptions: Default::default(),
        store: store.clone(),
    });

    let heard = Arc::new(Mutex::new(Vec::new()));
    let sink = heard.clone();
    app.listen_any("amethystate://todos:items", move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str::<serde_json::Value>(event.payload()).unwrap())
    });

    let state = app.state::<PluginState>();
    amethystate_subscribe(state.clone(), app.handle().clone(), "todos.items".into())
        .await
        .unwrap();

    let writer = uuid::Uuid::new_v4();
    store
        .set_with_source(
            StorePath::from_segments(["todos", "items", "a.b"]),
            &7u32,
            Some(writer),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));

    assert_eq!(
        *heard.lock().unwrap(),
        [serde_json::json!({ "type": "Insert", "key": "a.b", "value": 7, "source": writer })]
    );
}
