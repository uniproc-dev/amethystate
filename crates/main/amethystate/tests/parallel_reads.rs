//! Reading a map back on one core and on several gives the same map.
//!
//! The choice is a setting on the store rather than a build flag, so both
//! paths are in every binary and one test covers both. A flag would have
//! doubled the matrix instead, and the path nobody built would be the one that
//! rots.
//!
//! Sized past the point where the work is divided at all - below roughly a
//! thousand entries neither setting splits anything, and a test under that
//! would compare one code path with itself.
//!
//! redb by name, and only redb: `StoreBackend::parallel_reads` is implemented
//! there and nowhere else, so every other engine takes the trait's `false` and
//! both halves of these tests would run the same branch. Which is exactly the
//! failure the paragraph above says the setting was chosen to avoid, and it
//! was happening - the file named no backend, so under `--features json` it
//! compared one code path with itself after all.

#![cfg(feature = "redb")]

use amethystate::store::StoreBackend;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;

const ENTRIES: usize = 5_000;

/// Opens with the setting asked for, and checks the store agrees it got it.
///
/// Without this a builder that dropped the setting on the floor would leave
/// both halves sequential, agreeing perfectly.
fn opened_with(path: &TempPath, parallel: bool) -> amethystate::Store {
    let store = StoreBuilder::new(path.path())
        .backend(Backend::Redb)
        .parallel_reads(parallel)
        .build()
        .unwrap();

    assert_eq!(
        store.parallel_reads(),
        parallel,
        "the store was asked for parallel_reads={parallel} and reports otherwise, \
         so this test would be comparing one branch with itself"
    );
    store
}

fn key(i: usize) -> String {
    format!("k{i:05}")
}

#[test]
fn both_settings_read_back_the_same_map() {
    let path = TempPath::new("parallel_reads");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Redb)
            .build()
            .unwrap();
        let map = store.kv().map::<String, u64>("wide").unwrap();
        for i in 0..ENTRIES {
            map.insert(key(i), &(i as u64)).unwrap();
        }
        store.save_now().unwrap();
    }

    let sequential = {
        let store = opened_with(&path, false);
        let map = store.kv().map::<String, u64>("wide").unwrap();
        map.entries().collect::<Vec<_>>()
    };

    let parallel = {
        let store = opened_with(&path, true);
        let map = store.kv().map::<String, u64>("wide").unwrap();
        map.entries().collect::<Vec<_>>()
    };

    assert_eq!(sequential.len(), ENTRIES, "every entry came back");
    assert_eq!(
        sequential, parallel,
        "dividing the work across cores must not change what is read, nor the \
         order it is read in"
    );
}

/// A key one entry cannot read is still an error, and still names that entry,
/// whichever way the work was divided. Rayon reports one failure out of many,
/// so this is where "which one" could quietly become "some one".
#[test]
fn a_bad_entry_is_reported_either_way() {
    let path = TempPath::new("parallel_reads_bad");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Redb)
            .build()
            .unwrap();
        let map = store.kv().map::<String, u64>("wide").unwrap();
        for i in 0..ENTRIES {
            map.insert(key(i), &(i as u64)).unwrap();
        }
        store.save_now().unwrap();

        // One entry that will not read back as the map's value type.
        store
            .kv()
            .namespace("wide")
            .set("k00042", &"not a number".to_string())
            .unwrap();
        store.save_now().unwrap();
    }

    for parallel in [false, true] {
        let store = opened_with(&path, parallel);

        let failure = store
            .kv()
            .map::<String, u64>("wide")
            .expect_err("a value that will not decode is an error, not an absence");

        let text = format!("{failure:?}");
        assert!(
            text.contains("k00042"),
            "parallel = {parallel}: the failure should name the entry: {text}"
        );
    }
}
