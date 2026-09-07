use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::time::Duration;

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
mod common;

#[amethystate(prefix = "dbl")]
pub struct Cfg {
    #[amestate(default = 0u64)]
    pub counter: u64,
}

/// The debouncer clones the write buffer, releases the lock, commits, then
/// clears its snapshot's keys. Clearing them by name dropped a write that had
/// landed in between: subscribers had already been told about it, but it was
/// neither on disk nor still buffered, so nothing would ever write it.
///
/// Each round lands its second write while the first is being committed, then
/// stops - a dropped write only stays lost if nothing writes that key again.
/// Nothing is flushed explicitly, since flushing would write whatever is still
/// buffered and hide the loss.
#[backends(all)]
fn a_write_during_a_commit_is_not_dropped(backend: Backend) {
    let path = TempPath::new("debounce_loss");
    let store = StoreBuilder::new(&path)
        .backend(backend)
        .disk(|d| d.debounce(Duration::from_millis(25)))
        .build()
        .unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    for round in 1..=40u64 {
        let first = round * 10;
        cfg.counter().set(first).unwrap();
        std::thread::sleep(Duration::from_millis(20));

        let second = first + 1;
        cfg.counter().set(second).unwrap();
        std::thread::sleep(Duration::from_millis(60));

        assert_eq!(
            store.get::<u64>(["dbl", "counter"]).unwrap(),
            Some(second),
            "round {round}: the write that landed during the commit is gone"
        );
    }
}

#[backends(all)]
fn a_burst_of_writes_settles_on_the_last_one(backend: Backend) {
    let path = TempPath::new("debounce_burst");

    {
        let store = StoreBuilder::new(&path)
            .backend(backend)
            .disk(|d| d.debounce(Duration::from_millis(15)))
            .build()
            .unwrap();
        let cfg = Cfg::new_with(&store).unwrap();

        for n in 1..=300u64 {
            cfg.counter().set(n).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }

        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(store.get::<u64>(["dbl", "counter"]).unwrap(), Some(300));
    }

    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    assert_eq!(
        store.get::<u64>(["dbl", "counter"]).unwrap(),
        Some(300),
        "and it survives a reopen"
    );
}

/// The one place the debouncer's own timer is observed.
///
/// Everything else here reads through an open store - which answers from the
/// write buffer, whether or not anything reached the disk - or reads after the
/// store was dropped, and the closing flush runs whatever the timer did. Both
/// stay green with the timer arm removed entirely. This reads the file while
/// the store is still open and nobody has asked it to save.
///
/// Text only, because the file is the observation: a flat engine's bytes need
/// its own reader to say anything, and `durability_crash.rs` is where that is
/// done.
#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn the_debouncer_writes_the_file_with_nobody_asking() {
    let path = TempPath::new("debounce_unasked");

    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .disk(|d| d.debounce(Duration::from_millis(15)))
        .build()
        .unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    cfg.counter().set(4242).unwrap();

    let holds = |value: u64| {
        let text = std::fs::read_to_string(path.path()).unwrap_or_default();
        text.contains(&format!("\"counter\": {value}"))
            || text.contains(&format!("counter = {value}"))
    };

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let landed = loop {
        if holds(4242) {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert!(
        landed,
        "the quiet period passed and nothing wrote the file, so the debouncer \
         never fired: {}",
        std::fs::read_to_string(path.path()).unwrap_or_default()
    );
}

/// A short-lived process gets one chance to write what is still buffered, and
/// it is the drop. Every backend family flushes from its own `Drop`, so this
/// holds even while the quiet period has tens of seconds left to run.
#[backends(all)]
fn dropping_the_store_writes_what_is_still_buffered(backend: Backend) {
    let path = TempPath::new("debounce_drop");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .disk(|d| d.debounce(Duration::from_secs(30)))
            .build()
            .unwrap();
        let cfg = Cfg::new_with(&store).unwrap();
        cfg.counter().set(777).unwrap();
    }

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    assert_eq!(store.get::<u64>(["dbl", "counter"]).unwrap(), Some(777));
}
