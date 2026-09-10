#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::StoreBackend;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::collections::HashMap;

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
    std::thread::sleep(std::time::Duration::from_millis(120));
}

/// A single level named `.` is a path like any other - it joins to `\.`, which
/// is not the root's key. Reading it must not hand back the whole document.
#[test]
fn a_level_named_dot_is_not_the_document_root() {
    let path = TempPath::new("tamper_dot_read");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();

    let dot = StorePath::segment(".");
    assert_eq!(dot.to_string(), "\\.", "the level joins to an escaped dot");

    let raw = StoreBackend::get_raw(&store, &dot).unwrap();
    assert_eq!(
        raw, None,
        "nothing was ever written at a level named `.`, yet the store answers"
    );
}

/// Writing at a level named `.` must write there, not replace every value in
/// the store.
#[test]
fn writing_at_a_level_named_dot_does_not_wipe_the_store() {
    let path = TempPath::new("tamper_dot_write");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();

    store.set(["cfg", "width"], &1280u32).unwrap();
    store.set(["cfg", "height"], &720u32).unwrap();

    let mut payload: HashMap<String, u32> = HashMap::new();
    payload.insert("zzz".to_string(), 1);
    store.set(["."], &payload).unwrap();

    assert_eq!(
        store.get::<u32>(["cfg", "width"]).unwrap(),
        Some(1280),
        "a write at `.` destroyed an unrelated value"
    );
    assert_eq!(store.get::<u32>(["cfg", "height"]).unwrap(), Some(720));
}

/// The same through `Kv`, where the name comes from the caller's data - an
/// entry a user named `.` is all it takes.
#[test]
fn a_kv_entry_named_dot_does_not_wipe_the_store() {
    let path = TempPath::new("tamper_dot_kv");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();

    let kv = store.kv();
    kv.set("width", &1280u32).unwrap();

    let mut payload: HashMap<String, u32> = HashMap::new();
    payload.insert("zzz".to_string(), 1);
    kv.set(".", &payload).unwrap();

    assert_eq!(
        kv.get::<u32>("width").unwrap(),
        Some(1280),
        "an entry named `.` destroyed every other entry"
    );
}

/// And the loss has to survive a restart to count, so check the file too.
#[test]
fn a_write_at_a_level_named_dot_does_not_wipe_the_file() {
    let path = TempPath::new("tamper_dot_write_disk");

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["cfg", "width"], &1280u32).unwrap();
        let mut payload: HashMap<String, u32> = HashMap::new();
        payload.insert("zzz".to_string(), 1);
        store.set(["."], &payload).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    assert_eq!(
        store.get::<u32>(["cfg", "width"]).unwrap(),
        Some(1280),
        "the file no longer holds the value that was written before `.`"
    );
}

/// Deleting a level named `.` must remove that level, not silently do nothing.
#[test]
fn deleting_a_level_named_dot_removes_it() {
    let path = TempPath::new("tamper_dot_delete");
    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();

    let contents = doc! {
        json = "{\n  \".\": 7,\n  \"cfg.width\": 1280\n}\n",
        toml = "\".\" = 7\n\"cfg.width\" = 1280\n",
        ron  = "{\".\": 7, \"cfg.width\": 1280}",
    };
    std::fs::write(path.path(), contents).unwrap();
    drop(store);
    settle();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let dot = StorePath::segment(".");
    StoreBackend::delete(&store, &dot).unwrap();

    assert_eq!(
        StoreBackend::get_raw(&store, &dot).unwrap(),
        None,
        "the level named `.` is still there after deleting it"
    );
}

/// A key literally named `.` written by hand is an ordinary one-level name.
/// Reading it must give back what is stored under it, and a write must go back
/// under the name the person used rather than under the spelling this library
/// would have chosen.
#[test]
fn a_hand_written_dot_key_reads_back_its_own_value() {
    let path = TempPath::new("tamper_dot_hand");

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["cfg", "width"], &1280u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let contents = doc! {
        json = "{\n  \".\": 7,\n  \"cfg.width\": 1280\n}\n",
        toml = "\".\" = 7\n\"cfg.width\" = 1280\n",
        ron  = "{\".\": 7, \"cfg.width\": 1280}",
    };
    std::fs::write(path.path(), contents).unwrap();

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();

        assert_eq!(
            store.get::<u32>(["."]).unwrap(),
            Some(7),
            "the value under the hand-written `.` key is not what came back"
        );
        assert_eq!(
            store.get::<u32>(["cfg", "width"]).unwrap(),
            Some(1280),
            "and it does not disturb what is beside it"
        );

        store.set(["."], &9u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let after = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        !after.contains("\\."),
        "the write invented a second spelling instead of using the one in the file: {after}"
    );

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    assert_eq!(store.get::<u32>(["."]).unwrap(), Some(9));
}

/// Scanning must not report the whole document as the value of one entry.
#[test]
fn scanning_over_a_hand_written_dot_key_does_not_yield_the_document() {
    let path = TempPath::new("tamper_dot_scan");

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["cfg", "width"], &1280u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let contents = doc! {
        json = "{\n  \".\": 7,\n  \"cfg.width\": 1280\n}\n",
        toml = "\".\" = 7\n\"cfg.width\" = 1280\n",
        ron  = "{\".\": 7, \"cfg.width\": 1280}",
    };
    std::fs::write(path.path(), contents).unwrap();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let entries = StoreBackend::scan_prefix(&store, &StorePath::root()).unwrap();

    let dot_entry = entries
        .iter()
        .find(|(k, _)| k == &StorePath::segment("."))
        .map(|(_, v)| String::from_utf8_lossy(v).to_string());

    if let Some(rendered) = dot_entry {
        assert!(
            !rendered.contains("1280"),
            "the entry for the `.` key carries the whole document: {rendered}"
        );
    }
}
