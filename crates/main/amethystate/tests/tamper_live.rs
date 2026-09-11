#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::WhenItWillNotRead;
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
    json = "{ \"cfg.width\": 1280, \"cfg.note\": \"edited by hand\" }\n",
    toml = "\"cfg.width\" = 1280\n\"cfg.note\" = \"edited by hand\"\n",
    ron  = "{\"cfg.width\": 1280, \"cfg.note\": \"edited by hand\"}",
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
            Ok(())
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

const HALF_WRITTEN: &str = doc! {
    json = "{ \"cfg\": { \"width\": half-written by an editor",
    toml = "[cfg\nwidth = half-written by an editor",
    ron  = "{\"cfg\": {\"width\": half-written by an editor",
};

/// Opens a store, writes a key, then breaks the file from outside the way an
/// editor caught mid-keystroke would.
fn broken_under(rule: WhenItWillNotRead) -> (amethystate::Store, TempPath) {
    let path = TempPath::new("tamper_live_broken");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .when_it_will_not_read(rule)
        .disk(|d| {
            d.debounce(Duration::from_millis(20))
                .watch_every(Duration::from_millis(20))
        })
        .build()
        .unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();
    store.save_now().unwrap();
    settle();

    std::fs::write(path.path(), HALF_WRITTEN).unwrap();
    settle();

    (store, path)
}

#[test]
fn a_file_broken_for_a_moment_is_waited_out_rather_than_acted_on() {
    let (store, path) = broken_under(WhenItWillNotRead::TryAgainFor(Duration::from_secs(30)));

    store.set(["cfg", "width"], &1024u32).unwrap();

    assert!(
        store.save_now().is_err(),
        "the store decided about a file that had been unreadable for a moment"
    );

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains("half-written"),
        "the file was touched while the window still stood: {written}"
    );

    let aside = path.path().with_extension(format!(
        "{}.unreadable",
        path.path().extension().unwrap().to_string_lossy()
    ));
    assert!(
        !aside.exists(),
        "the file was set aside while it might still have fixed itself"
    );

    // And once it reads again the save lands, with the edit from outside in it.
    std::fs::write(path.path(), EDITED).unwrap();
    settle();
    store.save_now().unwrap();
    drop(store);
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        !written.contains("half-written"),
        "the save never came round again after the file healed: {written}"
    );
}

#[test]
fn a_file_that_stays_broken_past_the_window_is_set_aside() {
    let (store, path) = broken_under(WhenItWillNotRead::TryAgainFor(Duration::ZERO));

    store.set(["cfg", "width"], &1024u32).unwrap();
    store.save_now().unwrap();
    drop(store);
    settle();

    let aside = path.path().with_extension(format!(
        "{}.unreadable",
        path.path().extension().unwrap().to_string_lossy()
    ));
    assert!(
        std::fs::read_to_string(&aside)
            .unwrap_or_default()
            .contains("half-written"),
        "the window ran out and what was in the file was not kept"
    );
}

#[test]
fn a_broken_external_edit_is_set_aside_rather_than_written_over() {
    let (store, path) = broken_under(WhenItWillNotRead::SetAside);

    store.set(["cfg", "width"], &1024u32).unwrap();
    store.save_now().unwrap();
    drop(store);
    settle();

    let aside = path.path().with_extension(format!(
        "{}.unreadable",
        path.path().extension().unwrap().to_string_lossy()
    ));

    let kept = std::fs::read_to_string(&aside).unwrap_or_default();
    assert!(
        kept.contains("half-written"),
        "what the editor had half typed was not kept anywhere: {}",
        aside.display()
    );

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        !written.contains("half-written"),
        "the store was told to set the broken file aside and write, and did not write"
    );
}

#[test]
fn a_broken_external_edit_stops_a_save_where_that_was_asked_for() {
    let (store, path) = broken_under(WhenItWillNotRead::Refuse);

    store.set(["cfg", "width"], &1024u32).unwrap();

    assert!(
        store.save_now().is_err(),
        "the save went ahead over a file the store was told to leave alone"
    );

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains("half-written"),
        "the file was written over despite the refusal: {written}"
    );

    assert_eq!(
        store.get::<u32>(["cfg", "width"]).unwrap(),
        Some(1024),
        "what could not be saved was dropped from memory as well"
    );
}

#[test]
fn a_broken_external_edit_is_flattened_where_that_was_asked_for() {
    let (store, path) = broken_under(WhenItWillNotRead::Overwrite);

    store.set(["cfg", "width"], &1024u32).unwrap();
    store.save_now().unwrap();
    drop(store);
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        !written.contains("half-written"),
        "the store was told to write over an unreadable file and did not"
    );
}

/// An editor that truncates before writing leaves the file empty for a moment.
/// The watcher debounce exists so that moment is not read as the document; a
/// format that calls an empty file a valid empty document defeats it, and every
/// key is seen as deleted.
#[test]
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
