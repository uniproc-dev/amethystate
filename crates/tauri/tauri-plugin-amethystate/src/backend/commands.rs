use amethystate::StoreSubscription;
use amethystate::store::StoreBackend;
use amethystate::store::StorePath;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Runtime, State};

pub struct PluginState {
    pub store: amethystate::Store,
    pub subscriptions: Mutex<HashMap<String, Watched>>,
}

/// A path the frontend listens to, and how many of its listeners are left.
pub struct Watched {
    _listening: StoreSubscription,
    watchers: usize,
}

#[tauri::command]
pub async fn amethystate_get(
    store: State<'_, PluginState>,
    key: String,
) -> Result<Option<serde_json::Value>, String> {
    let path = StorePath::parse_joined(&key).map_err(|e| e.to_string())?;
    store.store.get(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn amethystate_set(
    store: State<'_, PluginState>,
    key: String,
    value: serde_json::Value,
    source: Option<uuid::Uuid>,
) -> Result<(), String> {
    let path = StorePath::parse_joined(&key).map_err(|e| e.to_string())?;
    store
        .store
        .set_with_source(&path, &value, source)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn amethystate_get_prefix(
    store: State<'_, PluginState>,
    prefix: String,
) -> Result<HashMap<String, serde_json::Value>, String> {
    let prefix = StorePath::parse_joined(&prefix).map_err(|e| e.to_string())?;
    let raw =
        amethystate::StoreBackend::scan_prefix(&store.store, &prefix).map_err(|e| e.to_string())?;

    let mut map = HashMap::new();
    for (path, bytes) in raw {
        if let Ok(val) = store.store.decode::<serde_json::Value>(&bytes) {
            map.insert(path.to_string(), val);
        }
    }
    Ok(map)
}

#[tauri::command]
pub async fn amethystate_flush(
    store: State<'_, PluginState>,
    prefix: String,
) -> Result<(), String> {
    let prefix = StorePath::parse_joined(&prefix).map_err(|e| e.to_string())?;
    amethystate::StoreBackend::flush_prefix(&store.store, &prefix).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn amethystate_subscribe<R: Runtime>(
    store: State<'_, PluginState>,
    app: AppHandle<R>,
    key: String,
) -> Result<(), String> {
    let mut subs = store.subscriptions.lock().map_err(|e| e.to_string())?;
    if let Some(watched) = subs.get_mut(&key) {
        watched.watchers += 1;
        return Ok(());
    }

    let app_handle = app.clone();
    let store_clone = store.store.clone();

    let prefix = amethystate::store::StorePath::parse_joined(&key).map_err(|e| e.to_string())?;

    let watched = prefix.clone();
    let event_name = amethystate::tauri::event_channel(&prefix);

    let listening = store.store.subscribe(
        amethystate::SubscriptionKind::Prefix(prefix),
        Arc::new(move |event| {
            let store_c = store_clone.clone();

            let source = match event.source {
                amethystate::reactive::Source::Handle(id) => Some(id),
                _ => None,
            };

            if matches!(event.op, amethystate::StoreOp::DeletePrefix)
                && watched.starts_with(&event.path)
            {
                let _ = app_handle.emit(
                    &event_name,
                    serde_json::json!({ "type": "Clear", "source": source }),
                );
                return Ok(());
            }

            let under = event
                .path
                .strip_prefix(&watched)
                .filter(|rest| rest.len() == 1);

            if let Some(rest) = under {
                let subkey = rest.segment_at(0).map(|level| level.as_str().to_string());
                let old_val = event
                    .old
                    .as_ref()
                    .and_then(|b| store_c.decode::<serde_json::Value>(b).ok());
                let new_val = event
                    .new
                    .as_ref()
                    .and_then(|b| store_c.decode::<serde_json::Value>(b).ok());

                let payload = match event.op {
                    amethystate::StoreOp::Set => {
                        if let Some(old) = old_val {
                            serde_json::json!({
                                "type": "Update",
                                "key": subkey,
                                "oldValue": old,
                                "newValue": new_val.unwrap_or(serde_json::Value::Null),
                                "source": source,
                            })
                        } else {
                            serde_json::json!({
                                "type": "Insert",
                                "key": subkey,
                                "value": new_val.unwrap_or(serde_json::Value::Null),
                                "source": source,
                            })
                        }
                    }
                    amethystate::StoreOp::Delete => serde_json::json!({
                        "type": "Remove",
                        "key": subkey,
                        "oldValue": old_val.unwrap_or(serde_json::Value::Null),
                        "source": source,
                    }),
                    amethystate::StoreOp::DeletePrefix => serde_json::json!({
                        "type": "Clear",
                        "source": source,
                    }),
                };
                let _ = app_handle.emit(&event_name, payload);
            } else if event.path == watched
                && let Some(new_bytes) = &event.new
                && let Ok(val) = store_c.decode::<serde_json::Value>(new_bytes)
            {
                let _ = app_handle.emit(&event_name, val);
            }

            Ok(())
        }),
    );

    subs.insert(
        key,
        Watched {
            _listening: listening,
            watchers: 1,
        },
    );
    Ok(())
}

#[tauri::command]
pub async fn amethystate_unsubscribe(
    state: State<'_, PluginState>,
    key: String,
) -> Result<(), String> {
    let mut subs = state.subscriptions.lock().map_err(|e| e.to_string())?;
    if let Some(watched) = subs.get_mut(&key) {
        watched.watchers -= 1;
        if watched.watchers == 0 {
            subs.remove(&key);
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn amethystate_delete(
    store: State<'_, PluginState>,
    key: String,
    source: Option<uuid::Uuid>,
) -> Result<(), String> {
    let path = StorePath::parse_joined(&key).map_err(|e| e.to_string())?;
    store
        .store
        .delete_with_source(&path, source)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn amethystate_delete_prefix(
    store: State<'_, PluginState>,
    prefix: String,
    source: Option<uuid::Uuid>,
) -> Result<(), String> {
    let prefix = StorePath::parse_joined(&prefix).map_err(|e| e.to_string())?;
    amethystate::StoreBackend::delete_prefix_with_source(&store.store, &prefix, source)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn amethystate_scan_keys(
    store: State<'_, PluginState>,
    prefix: String,
) -> Result<Vec<String>, String> {
    let prefix = StorePath::parse_joined(&prefix).map_err(|e| e.to_string())?;
    let keys =
        amethystate::StoreBackend::scan_keys(&store.store, &prefix).map_err(|e| e.to_string())?;
    Ok(keys.iter().map(|k| k.to_string()).collect())
}
