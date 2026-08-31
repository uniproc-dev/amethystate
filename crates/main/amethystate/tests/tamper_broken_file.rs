#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::StoreBuilder;
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

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

fn seeded(suffix: &str) -> TempPath {
    let path = TempPath::new(suffix);
    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["cfg", "width"], &1280u32).unwrap();
        store.set(["cfg", "height"], &720u32).unwrap();
        store.save_now().unwrap();
    }
    settle();
    path
}

fn meta_path(path: &Path) -> PathBuf {
    path.with_extension("meta")
}

/// Where the store keeps its copy: the whole name plus `.bak`.
fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

/// What a person might plausibly have put beside the store themselves, which
/// is what swapping the extension would have named.
fn a_neighbours_file(path: &Path) -> PathBuf {
    path.with_extension("bak")
}

/// A file the format cannot read must stop the store, and must still be there
/// afterwards - overwriting it with an empty document is the one outcome that
/// cannot be undone.
#[test]
fn a_truncated_file_is_refused_and_left_alone() {
    let path = seeded("tamper_truncated");
    let broken = doc! {
        json = "{ \"cfg\": { \"width\": 12",
        toml = "[cfg\nwidth = 12",
        ron  = "{\"cfg\": {\"width\": 12",
    };
    std::fs::write(path.path(), broken).unwrap();

    let opened = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build();
    assert!(opened.is_err(), "a truncated file opened without complaint");
    drop(opened);
    settle();

    assert_eq!(
        std::fs::read_to_string(path.path()).unwrap(),
        broken,
        "the unreadable file was rewritten"
    );
}

/// An empty file is not a document. Coming up empty and then saving over it is
/// how a config that was merely being written at the wrong moment is lost.
#[test]
#[cfg_attr(
    feature = "toml",
    ignore = "known: `TomlDocument::parse` has no root check, so an empty file is a valid empty document - see TODO.md"
)]
fn an_empty_file_is_refused() {
    let path = seeded("tamper_empty_file");
    std::fs::write(path.path(), "").unwrap();

    let opened = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build();
    assert!(
        opened.is_err(),
        "an empty file opened as an empty store, and the next save writes over it"
    );
}

/// A document whose root is not a table of names.
#[test]
fn a_scalar_root_is_refused() {
    let path = seeded("tamper_scalar_root");
    let broken = doc! {
        json = "42\n",
        toml = "42\n",
        ron  = "42",
    };
    std::fs::write(path.path(), broken).unwrap();

    let opened = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build();
    assert!(opened.is_err(), "a scalar root opened without complaint");
}

/// A top-level list where a table of names belongs. TOML has no such document,
/// so this asks only the two formats that do.
#[cfg(any(feature = "json", feature = "ron"))]
#[test]
fn an_array_root_is_refused() {
    let path = seeded("tamper_array_root");
    let broken = "[1, 2, 3]\n";
    std::fs::write(path.path(), broken).unwrap();

    let opened = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build();
    assert!(opened.is_err(), "an array root opened without complaint");
}

/// A file holding nothing but a comment is a document with no keys. Coming up
/// empty from one and then saving is how a config a person was in the middle of
/// commenting out is lost.
#[test]
#[cfg_attr(
    feature = "toml",
    ignore = "known: `TomlDocument::parse` has no root check, so a file with every key commented out is a valid empty document - see TODO.md"
)]
fn a_file_with_no_keys_left_is_refused() {
    let path = seeded("tamper_comment_only");
    let commented = doc! {
        json = "// everything commented out\n",
        toml = "# everything commented out\n",
        ron  = "// everything commented out\n",
    };
    std::fs::write(path.path(), commented).unwrap();

    let opened = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build();
    assert!(
        opened.is_err(),
        "a file with no keys in it opened as an empty store, and the next save \
         writes the store's own idea of the document over it"
    );
}

/// An open that gives up rolls both files back from the copies it took. Since
/// both copies were written to one path, the roll-back puts the metadata into
/// the data file and then deletes the metadata file, and the store the user had
/// is gone.
#[test]
fn an_open_that_gives_up_leaves_the_data_where_it_was() {
    let path = seeded("tamper_rollback");
    let data_before = std::fs::read_to_string(path.path()).unwrap();

    let opened =
        StoreBuilder::new(path.path())
            .backend(text_backend())
            .migrations(|m| {
                m.for_prefix("alpha")
                    .depends_on_raw("beta")
                    .step(1, "never runs", |_ctx| Ok(()));
                m.for_prefix("beta")
                    .depends_on_raw("alpha")
                    .step(1, "never runs either", |_ctx| Ok(()));
            })
            .build();

    assert!(
        opened.is_err(),
        "a dependency cycle between prefixes must refuse the open"
    );
    drop(opened);
    settle();

    assert_eq!(
        std::fs::read_to_string(path.path()).unwrap(),
        data_before,
        "the aborted open rewrote the data file"
    );
    assert!(
        meta_path(path.path()).exists(),
        "the aborted open deleted the metadata file"
    );
}

/// The store copies both files aside before touching them. The two copies must
/// not be the same file: `path.with_extension("bak")` is the same string for
/// `store.db` and for `store.meta`, so the metadata copy lands on top of the
/// data copy and the data has no backup left.
#[test]
fn the_data_and_metadata_backups_are_separate_files() {
    let path = seeded("tamper_backup_collision");

    let data_before = std::fs::read_to_string(path.path()).unwrap();
    let meta_before = std::fs::read_to_string(meta_path(path.path())).unwrap();
    assert_ne!(
        data_before, meta_before,
        "the two files differ to begin with"
    );

    std::fs::write(meta_path(path.path()), "not a document at all {{{").unwrap();

    let opened = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build();
    assert!(
        opened.is_err(),
        "an unreadable metadata file must be caught"
    );

    let bak = backup_path(path.path());
    assert!(bak.exists(), "the aborted open left no backup at all");

    assert_eq!(
        std::fs::read_to_string(&bak).unwrap(),
        data_before,
        "the only backup file holds the metadata, not the data"
    );
}

/// A backup path derived by swapping the extension names a file that may
/// already be there and is not the store's to touch.
#[test]
fn a_file_beside_the_store_is_left_alone() {
    let path = seeded("tamper_bak_squat");
    let bak = a_neighbours_file(path.path());
    std::fs::write(&bak, "the user's own backup").unwrap();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    store.set(["cfg", "width"], &1u32).unwrap();
    drop(store);
    settle();

    assert_eq!(
        std::fs::read_to_string(&bak).ok().as_deref(),
        Some("the user's own backup"),
        "opening the store destroyed an unrelated file sharing its stem"
    );
}
