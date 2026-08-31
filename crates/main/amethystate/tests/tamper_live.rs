#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::StoreBuilder;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

mod common;
use common::text_backend;

macro_rules! doc {
    (json = $j:expr, toml = $t:expr, ron = $r:expr $(,)?) => {{
        #[cfg(feature = "json")]
        {
            $j
        }
        #[cfg(all(feature = "toml", not(feature = "json")))]
        {
            $t
        }
        #[cfg(all(feature = "ron", not(feature = "json"), not(feature = "toml")))]
        {
            $r
        }
    }};
}

fn settle() {
    std::thread::sleep(Duration::from_millis(400));
}

const EDITED: &str = doc! {
    json = "{ \"cfg\": { \"width\": 1280, \"note\": \"edited by hand\" } }\n",
    toml = "[cfg]\nwidth = 1280\nnote = \"edited by hand\"\n",
    ron  = "{\"cfg\": {\"width\": 1280, \"note\": \"edited by hand\"}}",
};

/// An edit made to the file while the store is open reaches the store.
#[test]
fn an_external_edit_is_picked_up_while_the_store_is_open() {
    let path = TempPath::new("tamper_live_pickup");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .disk(|d| {
            d.debounce(Duration::from_millis(20))
                .watch_every(Duration::from_millis(20))
        })
        .build()
        .unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();
    store.set(["cfg", "note"], &"mine".to_string()).unwrap();
    store.save_now().unwrap();
    settle();

    std::fs::write(path.path(), EDITED).unwrap();
    settle();

    assert_eq!(
        store.get::<String>(["cfg", "note"]).unwrap(),
        Some("edited by hand".to_string()),
        "the watcher never picked the edit up"
    );
}

/// A subscriber has to hear about it too - a field bound to that path is
/// holding the old value otherwise.
#[test]
fn an_external_edit_notifies_a_subscriber() {
    use amethystate::{StoreBackend, SubscriptionKind};

    let path = TempPath::new("tamper_live_notify");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .disk(|d| {
            d.debounce(Duration::from_millis(20))
                .watch_every(Duration::from_millis(20))
        })
        .build()
        .unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();
    store.set(["cfg", "note"], &"mine".to_string()).unwrap();
    store.save_now().unwrap();
    settle();

    let hits = Arc::new(AtomicU32::new(0));
    let seen = hits.clone();
    StoreBackend::subscribe(
        &store,
        SubscriptionKind::ExactPath(StorePath::from_segments(["cfg", "note"])),
        Arc::new(move |_event| {
            seen.fetch_add(1, Ordering::Relaxed);
        }),
    );

    std::fs::write(path.path(), EDITED).unwrap();
    settle();

    assert!(
        hits.load(Ordering::Relaxed) > 0,
        "nothing told the subscriber the value on disk had changed"
    );
}

/// An edit to one key made outside the process, while the store has an unsaved
/// write to a different key, must not be thrown away: the store rewrites the
/// whole document from memory, so the edit is silently undone.
#[test]
#[ignore = "known: a save writes the whole document from memory rather than merging - see TODO.md"]
fn an_external_edit_survives_an_unrelated_pending_write() {
    let path = TempPath::new("tamper_live_conflict");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .disk(|d| {
            d.debounce(Duration::from_millis(400))
                .watch_every(Duration::from_millis(20))
        })
        .build()
        .unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();
    store.set(["cfg", "note"], &"mine".to_string()).unwrap();
    store.save_now().unwrap();
    settle();

    store.set(["cfg", "width"], &1024u32).unwrap();
    std::fs::write(path.path(), EDITED).unwrap();

    store.save_now().unwrap();
    drop(store);
    settle();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    assert_eq!(
        store.get::<u32>(["cfg", "width"]).unwrap(),
        Some(1024),
        "the store's own write was lost"
    );
    assert_eq!(
        store.get::<String>(["cfg", "note"]).unwrap(),
        Some("edited by hand".to_string()),
        "the edit to an untouched key was rolled back by the store's own save"
    );
}

/// A file the format cannot read, written while the store is open, is dropped
/// without a word and then overwritten by the next save. Whatever the user was
/// in the middle of writing is gone.
#[test]
#[ignore = "known: a failed reload returns early without a word, and the next save replaces the file - see TODO.md"]
fn a_broken_external_edit_is_not_silently_overwritten() {
    let path = TempPath::new("tamper_live_broken");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .disk(|d| {
            d.debounce(Duration::from_millis(20))
                .watch_every(Duration::from_millis(20))
        })
        .build()
        .unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();
    store.save_now().unwrap();
    settle();

    let half_written = doc! {
        json = "{ \"cfg\": { \"width\": half-written by an editor",
        toml = "[cfg\nwidth = half-written by an editor",
        ron  = "{\"cfg\": {\"width\": half-written by an editor",
    };
    std::fs::write(path.path(), half_written).unwrap();
    settle();

    store.set(["cfg", "width"], &1024u32).unwrap();
    store.save_now().unwrap();
    drop(store);
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains("half-written"),
        "the unreadable file was replaced without the caller ever seeing an error: {written}"
    );
}

/// An editor that truncates before writing leaves the file empty for a moment.
/// The watcher debounce exists so that moment is not read as the document; a
/// format that calls an empty file a valid empty document defeats it, and every
/// key is seen as deleted.
#[test]
#[cfg_attr(
    feature = "toml",
    ignore = "known: a truncated toml file parses as an empty document, so the watcher reads it as every key deleted - see TODO.md"
)]
fn a_momentarily_truncated_file_is_not_read_as_an_empty_store() {
    let path = TempPath::new("tamper_live_truncate");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .disk(|d| {
            d.debounce(Duration::from_millis(20))
                .watch_every(Duration::from_millis(20))
        })
        .build()
        .unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();
    store.set(["cfg", "note"], &"mine".to_string()).unwrap();
    store.save_now().unwrap();
    settle();

    std::fs::write(path.path(), "").unwrap();
    settle();

    assert_eq!(
        store.get::<u32>(["cfg", "width"]).unwrap(),
        Some(1280),
        "the store threw its whole document away over a truncated file"
    );
}

/// Two round trips, since one hides what the second shows: edit, reopen, write,
/// reopen.
#[test]
fn an_external_edit_survives_two_round_trips() {
    let path = TempPath::new("tamper_live_two_trips");

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["cfg", "width"], &1280u32).unwrap();
        store.set(["cfg", "note"], &"mine".to_string()).unwrap();
        store.save_now().unwrap();
    }
    settle();

    std::fs::write(path.path(), EDITED).unwrap();

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["cfg", "width"], &1024u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    assert_eq!(
        store.get::<String>(["cfg", "note"]).unwrap(),
        Some("edited by hand".to_string())
    );
    assert_eq!(store.get::<u32>(["cfg", "width"]).unwrap(), Some(1024));
}
