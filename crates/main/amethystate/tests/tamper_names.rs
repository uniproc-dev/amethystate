#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::StoreBackend;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;

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

fn seed(path: &std::path::Path, contents: &str) {
    {
        let store = StoreBuilder::new(path)
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["seed"], &1u32).unwrap();
        store.save_now().unwrap();
    }
    settle();
    std::fs::write(path, contents).unwrap();
}

/// A key with a dot in its name written by hand, beside a genuine two-level
/// nesting. Both are separate places and both must read back.
const DOTTED: &str = doc! {
    json = "{\n  \"cfg\": {\n    \"a.b\": 1,\n    \"a\": { \"b\": 2 }\n  }\n}\n",
    toml = "[cfg]\n\"a.b\" = 1\n\n[cfg.a]\nb = 2\n",
    ron  = "{\"cfg\": {\"a.b\": 1, \"a\": {\"b\": 2}}}",
};

#[test]
fn a_dotted_name_and_a_nesting_are_two_places() {
    let path = TempPath::new("tamper_dotted_read");
    seed(path.path(), DOTTED);

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    assert_eq!(
        store.get::<u32>(["cfg", "a.b"]).unwrap(),
        Some(1),
        "the hand-written `a.b` key"
    );
    assert_eq!(
        store.get::<u32>(["cfg", "a", "b"]).unwrap(),
        Some(2),
        "the genuine two-level nesting"
    );
}

/// Scanning a prefix must list the dotted name in its escaped form, which is
/// the only spelling that reads back as one level.
#[test]
fn a_scan_lists_the_dotted_name_escaped() {
    let path = TempPath::new("tamper_dotted_scan");
    seed(path.path(), DOTTED);

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let keys = StoreBackend::scan_keys(&store, &StorePath::segment("cfg")).unwrap();

    assert!(
        keys.contains(&StorePath::from_segments(["cfg", "a.b"])),
        "the dotted entry is missing from the scan: {keys:?}"
    );
}

/// Deleting a subtree must take a dotted name under it. The scan hands back the
/// escaped key, and anything that cuts that at the dot addresses a level which
/// is not there, so the delete removes nothing and still returns `Ok`.
#[test]
fn deleting_a_prefix_takes_the_dotted_name_with_it() {
    let path = TempPath::new("tamper_dotted_delete");
    seed(path.path(), DOTTED);

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    store.delete_prefix(["cfg"]).unwrap();

    assert_eq!(
        store.get::<u32>(["cfg", "a", "b"]).unwrap(),
        None,
        "the nested value survived the prefix delete"
    );
    assert_eq!(
        store.get::<u32>(["cfg", "a.b"]).unwrap(),
        None,
        "the dotted name survived the prefix delete"
    );
}

/// The same delete, checked after a restart: whatever the in-memory document
/// says, the file is what the next run reads.
#[test]
fn a_prefix_delete_over_a_dotted_name_survives_a_restart() {
    let path = TempPath::new("tamper_dotted_restart");
    seed(path.path(), DOTTED);

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.delete_prefix(["cfg"]).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    assert_eq!(
        store.get::<u32>(["cfg", "a.b"]).unwrap(),
        None,
        "the dotted name is back after a restart"
    );
}

/// A key with no name cannot be addressed, but it is in the document and the
/// store rewrites the document, so it must at least survive the round trip.
#[test]
fn a_key_with_no_name_survives_a_round_trip() {
    let path = TempPath::new("tamper_empty_name");
    let contents = doc! {
        json = "{\n  \"\": 9,\n  \"cfg\": { \"width\": 1280 }\n}\n",
        toml = "\"\" = 9\n\n[cfg]\nwidth = 1280\n",
        ron  = "{\"\": 9, \"cfg\": {\"width\": 1280}}",
    };
    seed(path.path(), contents);

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["cfg", "height"], &720u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains('9'),
        "the nameless key was dropped by the round trip: {written}"
    );
}

/// A name no path can hold costs the store nothing but that name.
///
/// A document may hold `{"": 1}`; a level with no name is not a path. The scan
/// passes over it - listing it would hand back a key that does not read back as
/// a path, and refusing would let one name nobody meant to write stop the store
/// from listing anything. What it may not do is take the value with it.
#[test]
fn a_key_with_no_name_costs_only_itself() {
    let path = TempPath::new("tamper_empty_scan");
    let contents = doc! {
        json = "{\n  \"\": 9,\n  \"cfg\": { \"width\": 1280 }\n}\n",
        toml = "\"\" = 9\n\n[cfg]\nwidth = 1280\n",
        ron  = "{\"\": 9, \"cfg\": {\"width\": 1280}}",
    };
    seed(path.path(), contents);

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();

    let keys = StoreBackend::scan_keys(&store, &StorePath::root()).unwrap();
    assert!(
        !keys.is_empty(),
        "one nameless key stopped the whole store from being listed"
    );
    for key in &keys {
        assert!(
            !key.is_root(),
            "the scan handed back {key:?}, which addresses nothing"
        );
    }
    assert_eq!(store.get::<u32>(["cfg", "width"]).unwrap(), Some(1280));

    store.set(["cfg", "width"], &800u32).unwrap();
    store.save_now().unwrap();
    drop(store);
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains("9"),
        "the value under the nameless key was dropped by a save: {written}"
    );
}
