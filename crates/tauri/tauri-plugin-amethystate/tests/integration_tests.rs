use amethystate::StoreBackend;
use amethystate::test_utils::unique_store;
use tauri_plugin_amethystate::backend::commands::PluginState;

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn test_tauri_plugin_commands() {
    use tauri::Manager;
    let store = unique_store("amethystate_tauri_test_store.redb");

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
