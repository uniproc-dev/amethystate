#![cfg(feature = "toml")]

use amethystate::amethystate;
use amethystate::store::StoreBackend;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::time::Duration;

#[amethystate(prefix = "cfg")]
pub struct Cfg {
    #[amestate(default = 0u32)]
    pub width: u32,

    #[amestate(default = 0u32)]
    pub height: u32,

    #[amestate(default = 0u32)]
    pub scale: u32,
}

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

/// A section is a table, and a table is not a number. Reading one back as a
/// scalar must say so rather than hand over the value of whichever key inside
/// it happens to come first.
#[test]
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

/// TOML's grammar calls an empty file a valid empty document, so a file caught
/// mid-write would read as "every key was deleted". Refusing it leaves the
/// store holding what it had, and the next save puts that back.
#[test]
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

/// A store that holds nothing leaves a file that holds nothing, and reads it
/// back. The file is the data and only the data: what the store thinks of
/// itself lives in the bookkeeping beside it.
#[test]
fn a_store_that_holds_nothing_leaves_an_empty_file_and_opens_again() {
    let path = TempPath::new("tamper_toml_nothing_stored");

    {
        StoreBuilder::new(path.path())
            .backend(Backend::Toml)
            .build()
            .unwrap();
    }
    settle();

    assert_eq!(
        std::fs::read_to_string(path.path()).unwrap().trim(),
        "",
        "a store with nothing in it wrote something into the file anyway"
    );

    let reopened = StoreBuilder::new(path.path())
        .backend(Backend::Toml)
        .build();
    assert!(
        reopened.is_ok(),
        "a store with nothing in it left a file it cannot read: {reopened:?}"
    );
}

/// The same, after a store that held something is emptied through the API: the
/// file goes back to nothing, and nothing is what it is read as.
#[test]
fn a_store_emptied_through_the_api_leaves_an_empty_file() {
    let path = TempPath::new("tamper_toml_cleared");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Toml)
            .build()
            .unwrap();
        store.set(["widths", "left"], &800u32).unwrap();
        store.save_now().unwrap();

        store.kv().clear().unwrap();
        store.save_now().unwrap();
    }
    settle();

    assert_eq!(
        std::fs::read_to_string(path.path()).unwrap().trim(),
        "",
        "clearing the store left something behind in the file"
    );

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Toml)
        .build()
        .expect("an emptied store is not a truncated one");

    assert_eq!(store.get::<u32>(["widths", "left"]).unwrap(), None);
}

/// An array of tables is a shape TOML has and the walker does not. A key
/// nothing declares is written whole at the root, beside it, so the array is
/// left where it stands.
#[test]
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
