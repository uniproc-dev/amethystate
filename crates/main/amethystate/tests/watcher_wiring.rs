#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::StoreBuilder;
use amethystate::{StoreBackend, SubscriptionKind};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

mod common;
use common::text_backend;

/// The file watcher reaches subscribers at all.
///
/// What the reread does with a changed file is pinned deterministically beside
/// the engines, by calling it. This is the other half - that an edit nobody in
/// this process made gets noticed - and it is the only test here that waits on
/// the operating system. The deadline is long enough that reaching it means the
/// event genuinely never arrived, and a quiet machine returns as soon as it
/// does.
#[test]
fn an_edit_from_outside_reaches_a_subscriber() {
    let path = TempPath::new("watcher_wiring");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .disk(|d| d.watch_every(Duration::from_millis(10)))
        .build()
        .unwrap();

    let key = StorePath::from_segments(["ui", "theme"]);
    store.set(&key, &"dark".to_string()).unwrap();
    store.save_now().unwrap();

    let (tx, rx) = mpsc::channel();
    store.subscribe(
        SubscriptionKind::ExactPath(key.clone()),
        Arc::new(move |event| {
            let _ = tx.send(event.source);
        }),
    );

    let on_disk = std::fs::read_to_string(path.path()).unwrap();
    std::fs::write(path.path(), on_disk.replace("dark", "light")).unwrap();

    let source = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("an edit made outside the process never reached a subscriber");

    assert_eq!(
        source,
        Some(amethystate::store::EXTERNAL_EDIT),
        "a change that came off the disk has to say so"
    );
    assert_eq!(store.get::<String>(&key).unwrap().as_deref(), Some("light"));
}
