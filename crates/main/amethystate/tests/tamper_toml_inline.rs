#![cfg(feature = "toml")]

use amethystate::store::StoreBackend;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::time::Duration;

fn settle() {
    std::thread::sleep(Duration::from_millis(120));
}

fn seeded(suffix: &str, contents: &str) -> TempPath {
    let path = TempPath::new(suffix);
    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Toml)
            .build()
            .unwrap();
        store.set(["seed"], &1u32).unwrap();
        store.save_now().unwrap();
    }
    settle();
    std::fs::write(path.path(), contents).unwrap();
    path
}

/// An inline table is how a person writes a small section by hand, and how many
/// tools emit one. Writing a new key into it must not empty it first.
#[test]
fn writing_into_an_inline_table_keeps_its_other_keys() {
    let path = seeded(
        "tamper_inline_write",
        "cfg = { width = 1280, height = 720 }\n",
    );

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Toml)
        .build()
        .unwrap();
    assert_eq!(
        store.get::<u32>(["cfg", "height"]).unwrap(),
        Some(720),
        "an inline table reads back before anything is written"
    );

    store.set(["cfg", "scale"], &2u32).unwrap();

    assert_eq!(
        store.get::<u32>(["cfg", "height"]).unwrap(),
        Some(720),
        "writing a sibling key emptied the inline table"
    );
    assert_eq!(store.get::<u32>(["cfg", "width"]).unwrap(), Some(1280));
}

/// The same loss, seen after the file is written back.
#[test]
fn an_inline_table_survives_a_restart() {
    let path = seeded(
        "tamper_inline_restart",
        "cfg = { width = 1280, height = 720 }\n",
    );

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Toml)
            .build()
            .unwrap();
        store.set(["cfg", "scale"], &2u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Toml)
        .build()
        .unwrap();
    assert_eq!(
        store.get::<u32>(["cfg", "height"]).unwrap(),
        Some(720),
        "the file no longer holds the other keys of the inline table"
    );
}

/// Deleting a key inside an inline table must remove it, not report success and
/// leave it there for the next run to read.
#[test]
fn a_key_inside_an_inline_table_can_be_deleted() {
    let path = seeded(
        "tamper_inline_delete",
        "cfg = { width = 1280, height = 720 }\n",
    );

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Toml)
        .build()
        .unwrap();
    StoreBackend::delete(&store, &StorePath::from_segments(["cfg", "width"])).unwrap();

    assert_eq!(
        store.get::<u32>(["cfg", "width"]).unwrap(),
        None,
        "the delete reported success and removed nothing"
    );
}

/// The bytes a node is handed to a deserializer as are `val = <node>`, and for
/// a node that is a table the code cuts that text at the first `=` and reads
/// the rest. For a table, the first `=` is the one inside it, so a section is
/// read back as the value of whichever key happens to come first.
#[test]
#[ignore = "known: `with_bytes_de` renders a non-value node and cuts at the first `=` - see TODO.md"]
fn a_section_is_not_read_back_as_one_of_its_own_keys() {
    let path = seeded("tamper_first_equals", "[cfg.width]\npx = 800\n");

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Toml)
        .build()
        .unwrap();
    let read = store.get::<u16>(["cfg", "width"]);

    assert!(
        read.is_err(),
        "a section was read as a number by taking a child's value: {read:?}"
    );
}

/// TOML calls an empty file a valid empty document, so a file caught mid-write
/// reads as "every key was deleted". The store then writes that back, and the
/// data is gone from disk with nothing to restore it from.
#[test]
#[ignore = "known: an empty toml file parses as a valid empty document - see TODO.md"]
fn a_momentary_truncation_is_not_written_back_as_the_document() {
    let path = TempPath::new("tamper_toml_truncate_persist");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Toml)
            .disk(|d| {
                d.debounce(Duration::from_millis(20))
                    .watch_every(Duration::from_millis(20))
            })
            .build()
            .unwrap();
        store.set(["cfg", "width"], &1280u32).unwrap();
        store.save_now().unwrap();
        std::thread::sleep(Duration::from_millis(400));

        std::fs::write(path.path(), "").unwrap();
        std::thread::sleep(Duration::from_millis(400));

        store.set(["other"], &1u32).unwrap();
        store.save_now().unwrap();
    }
    std::thread::sleep(Duration::from_millis(400));

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Toml)
        .build()
        .unwrap();
    assert_eq!(
        store.get::<u32>(["cfg", "width"]).unwrap(),
        Some(1280),
        "the file now holds the store's empty document instead of the data"
    );
}

/// An array of tables is a shape TOML has and the walker does not. Writing
/// beside it must not throw it away.
#[test]
#[ignore = "known: `as_table_like` does not reach an array of tables, so a write beside one is now refused rather than carried out - see TODO.md"]
fn an_array_of_tables_survives_a_write_beside_it() {
    let path = seeded(
        "tamper_aot",
        "[[servers]]\nname = \"a\"\n\n[[servers]]\nname = \"b\"\n",
    );

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Toml)
            .build()
            .unwrap();
        store.set(["servers", "count"], &2u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let written = std::fs::read_to_string(path.path()).unwrap();
    assert!(
        written.contains("\"a\""),
        "the array of tables was replaced: {written}"
    );
}
