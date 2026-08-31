pub mod document;
pub mod error;
mod inspector;
#[cfg(feature = "json")]
pub mod json;
pub mod migration;
#[cfg(feature = "ron")]
pub mod ron;
pub mod store;
#[cfg(feature = "toml")]
pub mod toml;

pub use document::TextDocument;
pub use error::TextStoreError;
pub use store::TextStore;

#[cfg(feature = "json")]
pub use json::JsonStore;

#[cfg(feature = "toml")]
pub use toml::TomlStore;

#[cfg(feature = "ron")]
pub use ron::RonStore;

#[macro_export]
macro_rules! define_store_test_suite {
    ($store_type:ident, $ext:expr, $watch_set_false:expr, $watch_set_true:expr, $watch_delete_empty:expr) => {
        #[cfg(test)]
        mod store_tests {
            use super::*;
            use std::path::PathBuf;
            use std::sync::Arc;
            use std::time::{SystemTime, UNIX_EPOCH};
            use $crate::store::config::StoreConfig;
            use $crate::store::{
                StoreBackend, StoreEvent, StoreExt, StoreOp, StorePath, SubscriptionKind,
            };

            fn unique_test_path(suffix: &str) -> PathBuf {
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time is after epoch")
                    .as_nanos();
                std::env::temp_dir().join(format!(
                    "amethystate-{}-{suffix}-{nanos}.{}",
                    stringify!($store_type),
                    $ext
                ))
            }

            fn open_store_at(path: PathBuf) -> $store_type {
                $store_type::open(StoreConfig::new(path), Default::default())
                    .unwrap()
                    .0
            }

            #[test]
            fn file_watch_emits_set_for_external_change() {
                let path = unique_test_path("watch-set");
                std::fs::write(&path, $watch_set_false).expect("seed file should be written");

                let store = open_store_at(path.clone());
                let (tx, rx) = std::sync::mpsc::channel::<StoreEvent>();

                store.subscribe(
                    SubscriptionKind::ExactPath(StorePath::from_segments([
                        "ui", "theme", "dark",
                    ])),
                    Arc::new(move |evt| {
                        let _ = tx.send(evt.clone());
                    }),
                );

                std::fs::write(&path, $watch_set_true).expect("updated file should be written");
                store.0.inner.pull_external_changes();

                let event = rx.try_recv().expect("the reread should emit a set event");

                assert_eq!(event.path.as_str(), "ui.theme.dark");
                assert_eq!(event.op, StoreOp::Set);
                let old_val: bool = store.decode(&event.old.as_ref().unwrap()).unwrap();
                let new_val: bool = store.decode(&event.new.as_ref().unwrap()).unwrap();
                assert_eq!(old_val, false);
                assert_eq!(new_val, true);
            }

            #[test]
            fn file_watch_emits_delete_for_external_removal() {
                let path = unique_test_path("watch-delete");
                std::fs::write(&path, $watch_set_true).expect("seed file should be written");

                let store = open_store_at(path.clone());
                let (tx, rx) = std::sync::mpsc::channel::<StoreEvent>();

                store.subscribe(
                    SubscriptionKind::ExactPath(StorePath::from_segments([
                        "ui", "theme", "dark",
                    ])),
                    Arc::new(move |evt| {
                        let _ = tx.send(evt.clone());
                    }),
                );

                std::fs::write(&path, $watch_delete_empty).expect("updated file should be written");
                store.0.inner.pull_external_changes();

                let event = rx.try_recv().expect("the reread should emit a delete event");

                assert_eq!(event.path.as_str(), "ui.theme.dark");
                assert_eq!(event.op, StoreOp::Delete);
                let old_val: bool = store.decode(&event.old.as_ref().unwrap()).unwrap();
                assert_eq!(old_val, true);
                assert_eq!(event.new, None);
            }

            #[test]
            fn save_now_and_persist() {
                let path = unique_test_path("save_now");
                let store = open_store_at(path.clone());

                store.set(["app", "version"], &"1.0.0".to_string()).unwrap();
                store.set(["app", "debug"], &true).unwrap();

                if path.exists() {
                    std::fs::remove_file(&path).unwrap();
                }

                store.save_now().unwrap();

                assert!(path.exists());
                let content = std::fs::read_to_string(&path).unwrap();
                assert!(content.contains("1.0.0"));
            }
        }
    };
}
