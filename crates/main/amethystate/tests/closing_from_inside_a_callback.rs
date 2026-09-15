//! Closing a store from inside something the store called.
//!
//! A subscriber runs on the thread that caused the change it is being told
//! about, and one of those threads is the file watcher's. Closing waits for
//! threads, so which thread a callback happens to be on decides whether a close
//! from inside it can be answered at all.

use amethystate::store::Flush;
use amethystate::store::builder::StoreBuilder;
use amethystate::store::config::AfterGivingUp;
use amethystate::store::field_with_path;
use amethystate::{StoreBackend, SubscriptionKind};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod common;

/// Long enough that a thread waiting for itself is told apart from one that is
/// merely slow.
const DEADLINE: Duration = Duration::from_secs(5);

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn a_subscriber_woken_by_an_outside_edit_may_close() {
    use common::text_backend;

    let path = TempPath::new("closing_from_a_watcher_callback");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .disk(|d| d.watch_every(Duration::from_millis(10)))
        .build()
        .unwrap();

    let theme =
        field_with_path::<String>(&store, ["ui", "theme"], "dark".into(), uuid::Uuid::new_v4())
            .unwrap();
    store.save_now().unwrap();

    let (tx, rx) = mpsc::channel::<&'static str>();
    let closing = store.clone();

    let _sub = theme.subscribe(move |_v: &String| {
        let _ = tx.send("entered");
        let _ = closing.close();
        let _ = tx.send("left");
    });

    let data = StoreBackend::files_layout(&store).unwrap().names()[0].clone();
    let on_disk = std::fs::read_to_string(&data).unwrap();
    std::fs::write(&data, on_disk.replace("dark", "light")).unwrap();

    assert_eq!(
        rx.recv_timeout(Duration::from_secs(30)).ok(),
        Some("entered"),
        "the subscription never fired, so nothing was tested"
    );
    assert!(
        rx.recv_timeout(DEADLINE).is_ok(),
        "the close never returned: the watcher's work runs on a thread closing waits for"
    );
}

/// The control: a subscriber the writing thread dispatched has always been able
/// to close, because the threads a close waits for are not this one.
#[test]
fn a_subscriber_the_writer_woke_may_close() {
    let path = TempPath::new("closing_from_a_writer_callback");
    let store = StoreBuilder::new(path.path()).build().unwrap();

    let key = StorePath::from_segments(["ui", "theme"]);
    let (tx, rx) = mpsc::channel::<&'static str>();
    let closing = store.clone();

    store.subscribe(
        SubscriptionKind::ExactPath(key.clone()),
        Arc::new(move |_event| {
            let _ = closing.close();
            let _ = tx.send("left");
            Ok(())
        }),
    );

    let _ = store.set(&key, &"light".to_string());

    assert!(
        rx.recv_timeout(DEADLINE).is_ok(),
        "the close never returned"
    );
}

/// `on_persist_failure` is the one callback that still runs on a thread closing
/// waits for, and the caller put it there. It is turned down with a sentence
/// rather than waited on.
#[cfg(feature = "json")]
#[test]
fn the_failure_callback_is_told_it_cannot_close() {
    use amethystate::store::builder::Backend;

    let dir = std::env::temp_dir().join(format!("ame_reentrant_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("store.json");

    let (tx, rx) = mpsc::channel::<Option<bool>>();
    let held: Arc<Mutex<Option<amethystate::Store>>> = Arc::new(Mutex::new(None));
    let from_callback = held.clone();

    let store = StoreBuilder::new(&file)
        .backend(Backend::Json)
        .disk(|d| {
            d.debounce(Duration::from_millis(10))
                .retry_every(Duration::from_millis(10))
                .give_up_after(Duration::from_millis(30))
                .on_failure(move |_reason| {
                    let told = from_callback
                        .lock()
                        .unwrap()
                        .clone()
                        .map(|store| store.close())
                        .and_then(Result::err)
                        .map(|why| matches!(why, Flush::Reentrant));
                    let _ = tx.send(told);
                    AfterGivingUp::Ignore
                })
        })
        .build()
        .unwrap();

    *held.lock().unwrap() = Some(store.clone());

    store.set(StorePath::from_segments(["a"]), &1u8).unwrap();
    store.save_now().unwrap();

    // A directory where the file goes, so every later rename fails.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let _ = store.set(StorePath::from_segments(["b"]), &2u8);

    let told = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the failure callback never ran, so nothing was tested");

    assert_eq!(
        told,
        Some(true),
        "closing from on_persist_failure should be turned down, not waited on"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
