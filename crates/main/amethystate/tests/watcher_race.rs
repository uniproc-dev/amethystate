#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::amethystate;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::test_utils::TempPath;
use std::time::Duration;

mod common;
use common::text_backend;

#[amethystate(prefix = "race")]
pub struct Cfg {
    #[amestate(default = 0u64)]
    pub counter: u64,
}

/// Whether the document on disk spells the counter at `value`, named rather
/// than searched for as a bare number - a file holding `1200` contains `200`.
fn file_holds(path: &std::path::Path, value: u64) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.contains(&format!("\"counter\": {value}")) || text.contains(&format!("counter = {value}"))
}

/// The watcher used to check "do we have unsaved changes" and only then take
/// the document lock, so a write landing in that gap was overwritten with what
/// had been read from disk - and subscribers were told about the rollback.
///
/// Writing steadily while the watcher runs puts writes into that window; every
/// one of them must survive.
#[test]
fn a_write_is_never_rolled_back_by_the_watcher() {
    let path = TempPath::new("watcher_race");
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
///
/// The file is read while the store is still open and nothing has asked it to
/// save. Waiting for the drop instead would prove nothing: `close` runs
/// `save_now` whatever the generation says, so a persist that marked a write
/// saved without writing it would still be covered by the closing flush.
#[test]
fn a_write_during_a_persist_still_reaches_the_file() {
    let path = TempPath::new("watcher_persist");

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

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let landed = loop {
        if file_holds(path.path(), 200) {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(
        landed,
        "writes stopped and the quiet period passed, and the file never took \
         the last one - a persist marked it saved without writing it: {}",
        std::fs::read_to_string(path.path()).unwrap_or_default()
    );
}
