#![cfg(feature = "redb")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::config::AfterGivingUp;
use amethystate::store::{OnUnreadable, UnreadableEntries, WillNotOpen};
use amethystate::{ReactiveMap, Store, amethystate};
use amethystate_core::test_utils::TempPath;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    signature: u8,
    is_windows_process: bool,
    display_name: String,
    size: u64,
    modified_ms: u64,
}

#[amethystate(prefix = "signatures")]
pub struct Cache {
    #[amestate(default = {})]
    pub verdicts: ReactiveMap<String, Verdict>,
}

fn open(path: &Path) -> Store {
    StoreBuilder::new(path)
        .backend(Backend::Redb)
        .when_it_will_not_open(WillNotOpen::StartFresh)
        .rules(|r| {
            r.on_unreadable(OnUnreadable::UseDefault)
                .unreadable_entries(UnreadableEntries::Skip)
        })
        .disk(|d| {
            d.debounce(Duration::from_secs(5))
                .on_failure(|_| AfterGivingUp::Ignore)
        })
        .build()
        .unwrap()
}

fn unnamed(held: &TempPath, name: &str) -> PathBuf {
    held.path().parent().unwrap().join(name)
}

fn verdict(name: &str) -> Verdict {
    Verdict {
        signature: 7,
        is_windows_process: true,
        display_name: name.to_string(),
        size: 4096,
        modified_ms: 1_700_000_000_000,
    }
}

#[test]
fn a_write_outlives_a_reopen_despite_the_long_debounce() {
    let held = TempPath::new("cache_reopen");
    let path = unnamed(&held, "cache");

    {
        let store = open(&path);
        let cache = Cache::new_with(&store).unwrap();
        cache
            .verdicts()
            .insert("k".to_string(), &verdict("kept"))
            .unwrap();
    }

    assert!(
        path.with_extension("redb").is_file(),
        "the engine did not name the file cache.redb"
    );

    let store = open(&path);
    let cache = Cache::new_with(&store).unwrap();
    assert_eq!(cache.verdicts().get("k"), Some(verdict("kept")));
}

#[test]
fn a_file_that_is_not_a_database_is_replaced_by_an_empty_store() {
    let held = TempPath::new("cache_garbage");
    let path = unnamed(&held, "cache");
    std::fs::write(
        path.with_extension("redb"),
        b"this was never a redb database, only bytes in its place",
    )
    .unwrap();

    let store = open(&path);
    let cache = Cache::new_with(&store).unwrap();
    assert_eq!(cache.verdicts().len(), 0);

    cache
        .verdicts()
        .insert("fresh".to_string(), &verdict("fresh"))
        .unwrap();
    assert_eq!(cache.verdicts().get("fresh"), Some(verdict("fresh")));
}

#[test]
fn an_entry_that_will_not_decode_is_left_out() {
    let held = TempPath::new("cache_bad_entry");
    let path = unnamed(&held, "cache");

    {
        let store = open(&path);
        {
            let cache = Cache::new_with(&store).unwrap();
            cache
                .verdicts()
                .insert("good".to_string(), &verdict("good"))
                .unwrap();
            cache
                .verdicts()
                .insert("bad".to_string(), &verdict("bad"))
                .unwrap();
        }
        store
            .set(
                ["signatures", "verdicts", "bad"],
                &"not a verdict".to_string(),
            )
            .unwrap();
    }

    let store = open(&path);
    let cache = Cache::new_with(&store).unwrap();
    assert_eq!(cache.verdicts().get("good"), Some(verdict("good")));
    assert_eq!(cache.verdicts().get("bad"), None);
}
