#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::amethystate;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::test_utils::unique_path;
use std::time::Duration;

mod common;
use common::text_backend;

#[amethystate(prefix = "race")]
pub struct Cfg {
    #[amestate(default = 0u64)]
    pub counter: u64,
}

/// The watcher used to check "do we have unsaved changes" and only then take
/// the document lock, so a write landing in that gap was overwritten with what
/// had been read from disk - and subscribers were told about the rollback.
///
/// Writing steadily while the watcher runs puts writes into that window; every
/// one of them must survive.
#[test]
fn a_write_is_never_rolled_back_by_the_watcher() {
    let path = unique_path("watcher_race");
    let store = StoreBuilder::new(&path)
        .backend(text_backend())
        .disk(|d| {
            d.debounce(Duration::from_millis(5))
                .watch_every(Duration::from_millis(5))
        })
        .build()
        .unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    for n in 1..=400u64 {
        cfg.counter().set(n).unwrap();

        let seen = cfg.counter().get();
        assert!(
            seen >= n,
            "write {n} was rolled back to {seen} by the watcher"
        );

        if n % 50 == 0 {
            std::thread::sleep(Duration::from_millis(12));
        }
    }

    store.save_now().unwrap();
    assert_eq!(store.get::<u64>(["race", "counter"]).unwrap(), Some(400));
}

/// A persist that runs while a write lands must not mark that write saved. The
/// generation is read before serializing, so a later write leaves the document
/// pending and the next persist picks it up.
#[test]
fn a_write_during_a_persist_still_reaches_the_file() {
    let path = unique_path("watcher_persist");

    {
        let store = StoreBuilder::new(&path)
            .backend(text_backend())
            .disk(|d| {
                d.debounce(Duration::from_millis(10))
                    .watch_every(Duration::from_millis(5))
            })
            .build()
            .unwrap();
        let cfg = Cfg::new_with(&store).unwrap();

        for n in 1..=200u64 {
            cfg.counter().set(n).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }

        std::thread::sleep(Duration::from_millis(200));
    }

    let store = StoreBuilder::new(&path)
        .backend(text_backend())
        .build()
        .unwrap();
    assert_eq!(
        store.get::<u64>(["race", "counter"]).unwrap(),
        Some(200),
        "the last write must have reached the file"
    );
}
