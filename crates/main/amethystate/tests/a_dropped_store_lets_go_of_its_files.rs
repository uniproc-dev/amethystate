#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::time::Duration;

mod common;

fn opened(backend: Backend, at: &TempPath) -> amethystate::Store {
    StoreBuilder::new(at.path())
        .backend(backend)
        .disk(|d| {
            d.debounce(Duration::from_secs(60))
                .watch_every(Duration::from_secs(60))
        })
        .build()
        .unwrap()
}

#[cfg(windows)]
#[test]
fn a_store_that_was_only_dropped_lets_the_platform_have_the_file() {
    let backend = common::text_backend();
    let at = TempPath::new("dropped_releases");

    let store = opened(backend, &at);
    store.set(["a"], &1u8).unwrap();
    store.save_now().unwrap();
    drop(store);

    let removed = std::fs::remove_file(at.path());
    assert!(
        removed.is_ok(),
        "a store nobody closed still held its file after it was dropped: {removed:?}"
    );
}

#[test]
fn a_store_written_to_and_dropped_does_not_hold_the_watcher_past_itself() {
    let backend = common::text_backend();

    for round in 0..400 {
        let at = TempPath::new(&format!("dropped_churn_{round}"));
        let store = opened(backend, &at);
        store.set(["a"], &1u8).unwrap();
        drop(store);
    }
}
