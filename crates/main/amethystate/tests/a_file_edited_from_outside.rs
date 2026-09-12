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
    TempPath,
    amethystate::Store,
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

    (path, store, rx)
}

#[backends(text)]
fn a_changed_value_reaches_a_subscriber_as_a_set(backend: Backend) {
    let file = edits(backend);
    let (path, store, rx) = watching(backend, "outside_set", file.holding_false);

    std::fs::write(path.path(), file.holding_true).unwrap();
    store.reread_from_disk();

    let event = rx.try_recv().expect("the reread emits a set");

    assert_eq!(event.path.to_string(), "ui.theme.dark");
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
    let (path, store, rx) = watching(backend, "outside_delete", file.holding_true);

    std::fs::write(path.path(), file.holding_nothing).unwrap();
    store.reread_from_disk();

    let event = rx.try_recv().expect("the reread emits a delete");

    assert_eq!(event.path.to_string(), "ui.theme.dark");
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

/// An edit that is undone is an edit.
///
/// A store remembers the bytes it last agreed with the file about, so that a
/// file merely touched costs no parse. Taking somebody else's edit is agreeing
/// with the file just as surely as writing it - and a store that forgets to say
/// so goes on holding *their* first edit for ever: they write, we take it, they
/// undo, and the file comes back to bytes we once wrote, which we wave past
/// without looking.
#[backends(text)]
fn an_edit_that_is_undone_reaches_a_subscriber(backend: Backend) {
    let file = edits(backend);
    let (path, store, rx) = watching(backend, "outside_undone", file.holding_false);

    // Ours, and saved, so the bytes below are bytes this store itself wrote.
    store.set(["ui", "theme", "dark"], &false).unwrap();
    store.flush_prefix(StorePath::root()).unwrap();
    let ours = std::fs::read_to_string(path.path()).unwrap();
    while rx.try_recv().is_ok() {}

    std::fs::write(path.path(), file.holding_true).unwrap();
    store.reread_from_disk();
    assert_eq!(
        store.get::<bool>(["ui", "theme", "dark"]).unwrap(),
        Some(true),
        "the outside edit was not taken"
    );
    while rx.try_recv().is_ok() {}

    std::fs::write(path.path(), &ours).unwrap();
    store.reread_from_disk();

    assert_eq!(
        store.get::<bool>(["ui", "theme", "dark"]).unwrap(),
        Some(false),
        "the file went back to what this store once wrote, and the store did not look"
    );
    assert!(
        rx.try_recv().is_ok(),
        "nobody was told the edit had been undone"
    );
}
