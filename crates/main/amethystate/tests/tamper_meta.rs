#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::StoreBuilder;
use amethystate::store::reactive_map_with_path_only;
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use std::collections::HashMap;
use uuid::Uuid;

mod common;
use common::text_backend;

mod v1 {
    use super::*;
    #[amethystate(prefix = "replay", version = 1)]
    pub struct Doc {
        #[amestate(default = 0u32)]
        pub hits: u32,
    }
}

#[amethystate(prefix = "replay", version = 2)]
pub struct Doc {
    #[amestate(default = 0u32)]
    pub hits: u32,
}

#[migrate]
fn double_the_hits(old: AmeData<v1::Doc>) -> amethystate::MigrationResult<AmeData<Doc>> {
    Ok(AmeData::<Doc> { hits: old.hits * 2 })
}

fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(120));
}

fn meta_path(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension("meta")
}

fn defaults() -> HashMap<String, u32> {
    let mut d = HashMap::new();
    d.insert("shipped".to_string(), 1);
    d
}

fn open_map(store: &amethystate::Store) -> amethystate::ReactiveMap<String, u32> {
    reactive_map_with_path_only::<String, u32>(store, ["items"], defaults(), Uuid::new_v4())
        .unwrap()
}

/// The store records "this namespace has been seeded" in the metadata file. If
/// that file is lost, the defaults must not come back over entries the user
/// deliberately removed.
#[test]
#[ignore = "known: nothing binds the metadata file to the data - see TODO.md"]
fn losing_the_metadata_file_does_not_resurrect_removed_defaults() {
    let path = TempPath::new("tamper_init_lost");

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        let map = open_map(&store);
        assert_eq!(map.get("shipped"), Some(1));
        map.remove("shipped").unwrap();
        drop(map);
        store.save_now().unwrap();
    }
    settle();

    std::fs::remove_file(meta_path(path.path())).unwrap();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let map = open_map(&store);
    assert_eq!(
        map.get("shipped"),
        None,
        "the entry the user removed came back when the metadata file went missing"
    );
}

/// The marker is a plain key in a file the store also rewrites. Forging it must
/// not be enough to make a fresh store come up without its defaults.
#[test]
#[ignore = "known: nothing binds the metadata file to the data - see TODO.md"]
fn a_forged_marker_does_not_suppress_the_defaults() {
    let path = TempPath::new("tamper_init_forged");

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        store.set(["unrelated"], &1u32).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let forged = {
        #[cfg(feature = "json")]
        {
            "{ \"__init.items\": true }\n"
        }
        #[cfg(all(feature = "toml", not(feature = "json")))]
        {
            "\"__init.items\" = true\n"
        }
        #[cfg(all(feature = "ron", not(feature = "json"), not(feature = "toml")))]
        {
            "{\"__init.items\": true}"
        }
    };
    std::fs::write(meta_path(path.path()), forged).unwrap();

    let store = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build()
        .unwrap();
    let map = open_map(&store);
    assert_eq!(
        map.get("shipped"),
        Some(1),
        "a forged marker in the metadata file suppressed the map's defaults"
    );
}

/// The metadata file also holds the schema version each prefix reached. Losing
/// it must not run an already-applied migration a second time over live data.
#[test]
#[ignore = "known: nothing binds the metadata file to the data - see TODO.md"]
fn losing_the_metadata_file_does_not_replay_a_migration() {
    let path = TempPath::new("tamper_replay");

    {
        let store = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build()
            .unwrap();
        let doc = v1::Doc::new_with(&store).unwrap();
        doc.hits().set(21).unwrap();
        drop(doc);
        store.save_now().unwrap();
    }
    settle();

    {
        let (store, _) = StoreBuilder::new(path.path())
            .backend(text_backend())
            .build_with_migration()
            .unwrap();
        assert_eq!(
            store.get::<u32>(["replay", "hits"]).unwrap(),
            Some(42),
            "the migration ran once"
        );
        store.save_now().unwrap();
    }
    settle();

    std::fs::remove_file(meta_path(path.path())).unwrap();

    let (store, _) = StoreBuilder::new(path.path())
        .backend(text_backend())
        .build_with_migration()
        .unwrap();
    assert_eq!(
        store.get::<u32>(["replay", "hits"]).unwrap(),
        Some(42),
        "the migration ran again over data it had already migrated"
    );
}
