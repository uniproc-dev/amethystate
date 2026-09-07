#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{StoreBackend, StoreOp, SubscriptionKind};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

struct Edits {
    holding_false: &'static str,
    holding_true: &'static str,
    holding_nothing: &'static str,
}

fn edits(backend: Backend) -> Edits {
    match backend {
        #[cfg(feature = "json")]
        Backend::Json => Edits {
            holding_false: r#"{"kept": 1, "ui.theme.dark": false}"#,
            holding_true: r#"{"kept": 1, "ui.theme.dark": true}"#,
            holding_nothing: r#"{"kept": 1}"#,
        },
        #[cfg(feature = "ron")]
        Backend::Ron => Edits {
            holding_false: "{\n  \"kept\": 1,\n  \"ui.theme.dark\": false,\n}",
            holding_true: "{\n  \"kept\": 1,\n  \"ui.theme.dark\": true,\n}",
            holding_nothing: "{\n  \"kept\": 1,\n}",
        },
        #[cfg(feature = "toml")]
        Backend::Toml => Edits {
            holding_false: "kept = 1\n\"ui.theme.dark\" = false\n",
            holding_true: "kept = 1\n\"ui.theme.dark\" = true\n",
            holding_nothing: "kept = 1\n",
        },
        other => panic!("{other:?} keeps no file a person could edit"),
    }
}

fn watching(
    backend: Backend,
    tag: &str,
    seeded_with: &str,
) -> (
    amethystate::Store,
    TempPath,
    Receiver<amethystate::StoreEvent>,
) {
    let path = TempPath::new(tag);
    std::fs::write(path.path(), seeded_with).expect("the seed file");

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let (tx, rx) = mpsc::channel();
    store.subscribe(
        SubscriptionKind::ExactPath(StorePath::from_segments(["ui", "theme", "dark"])),
        Arc::new(move |event| {
            let _ = tx.send(event.clone());
            Ok(())
        }),
    );

    (store, path, rx)
}

#[backends(text)]
fn a_changed_value_reaches_a_subscriber_as_a_set(backend: Backend) {
    let file = edits(backend);
    let (store, path, rx) = watching(backend, "outside_set", file.holding_false);

    std::fs::write(path.path(), file.holding_true).unwrap();
    store.reread_from_disk();

    let event = rx.try_recv().expect("the reread emits a set");

    assert_eq!(event.path.as_str(), "ui.theme.dark");
    assert_eq!(event.op, StoreOp::Set);
    assert_eq!(
        (
            store.decode::<bool>(event.old.as_ref().unwrap()).unwrap(),
            store.decode::<bool>(event.new.as_ref().unwrap()).unwrap(),
        ),
        (false, true)
    );
}

#[backends(text)]
fn a_removed_value_reaches_a_subscriber_as_a_delete(backend: Backend) {
    let file = edits(backend);
    let (store, path, rx) = watching(backend, "outside_delete", file.holding_true);

    std::fs::write(path.path(), file.holding_nothing).unwrap();
    store.reread_from_disk();

    let event = rx.try_recv().expect("the reread emits a delete");

    assert_eq!(event.path.as_str(), "ui.theme.dark");
    assert_eq!(event.op, StoreOp::Delete);
    assert!(store.decode::<bool>(event.old.as_ref().unwrap()).unwrap());
    assert_eq!(event.new, None);
}

#[backends(text)]
fn a_save_writes_the_file_the_store_was_opened_at(backend: Backend) {
    let path = TempPath::new("outside_save");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["app", "version"], &"1.0.0".to_string()).unwrap();
    std::fs::remove_file(path.path()).unwrap();

    store.save_now().unwrap();

    assert!(
        std::fs::read_to_string(path.path())
            .unwrap()
            .contains("1.0.0"),
        "the file the store names must hold what was written"
    );
}
