pub mod commands;

use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

pub fn init<R: Runtime>(store: amethystate::Store) -> TauriPlugin<R> {
    Builder::new("amethystate")
        .invoke_handler(tauri::generate_handler![
            commands::amethystate_get,
            commands::amethystate_set,
            commands::amethystate_subscribe,
            commands::amethystate_unsubscribe,
            commands::amethystate_get_prefix,
            commands::amethystate_flush,
            commands::amethystate_delete,
            commands::amethystate_delete_prefix,
            commands::amethystate_scan_keys,
        ])
        .setup(|app, _api| {
            app.manage(commands::PluginState {
                store,
                subscriptions: std::sync::Mutex::new(std::collections::HashMap::new()),
            });
            Ok(())
        })
        .build()
}
