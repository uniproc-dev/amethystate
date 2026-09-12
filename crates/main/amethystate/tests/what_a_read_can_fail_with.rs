use amethystate::Store;
use amethystate::store::builder::StoreBuilder;
use amethystate::store::{Flush, OpenStore, ReadValue, ScanKeys};
use amethystate_core::test_utils::TempPath;
use std::error::Error;

fn store(name: &str) -> (TempPath, Store) {
    let at = TempPath::new(name);
    let store = StoreBuilder::new(at.path()).build().unwrap();
    (at, store)
}

fn every_way_a_read_can_fail(why: ReadValue) -> String {
    match why {
        ReadValue::NotAPath(said) => format!("no path to look at: {said}"),
        ReadValue::WillNotRead { at, why } => {
            format!("{at} will not read: {}", why.current_context())
        }
        ReadValue::Closed { at } => format!("{at} is behind a closed store"),
        ReadValue::Store(why) => format!("the store: {}", why.current_context()),
    }
}

fn every_way_a_scan_can_fail(why: ScanKeys) -> String {
    match why {
        ScanKeys::NotAPath(said) => format!("no prefix to list: {said}"),
        ScanKeys::KeyWillNotRead { under, why } => {
            format!(
                "a key under {under} is not a path: {}",
                why.current_context()
            )
        }
        ScanKeys::Closed { under } => format!("{under} is behind a closed store"),
        ScanKeys::Store(why) => format!("the store: {}", why.current_context()),
    }
}

fn every_way_a_flush_can_fail(why: Flush) -> String {
    match why {
        Flush::Closed => "already closed".to_string(),
        Flush::Reentrant => "asked for from inside the store".to_string(),
        Flush::DidNotLand { why } => format!("did not land: {}", why.current_context()),
    }
}

fn every_way_an_open_can_fail(why: OpenStore) -> String {
    match why {
        OpenStore::WouldNotOpen { why } => format!("would not open: {}", why.current_context()),
        OpenStore::Migrating { why } => format!("would not migrate: {}", why.current_context()),
        OpenStore::Store(why) => format!("the store: {}", why.current_context()),
    }
}

#[test]
fn a_match_over_every_way_a_read_fails_needs_no_catch_all() {
    let (_at, store) = store("reads_exhaustive");

    store.set(["port"], &"not a number".to_string()).unwrap();
    let refused = store.get::<u16>(["port"]).unwrap_err();

    assert!(
        every_way_a_read_can_fail(refused).contains("port"),
        "the arm that ran must still name the place"
    );
}

#[test]
fn a_match_over_every_way_a_scan_fails_needs_no_catch_all() {
    let (_at, store) = store("scans_exhaustive");

    store.close().unwrap();
    let refused = store.scan_keys(["ui"]).unwrap_err();

    assert_eq!(
        every_way_a_scan_can_fail(refused),
        "ui is behind a closed store"
    );
}

#[test]
fn a_match_over_every_way_a_flush_fails_needs_no_catch_all() {
    let (_at, store) = store("flushes_exhaustive");

    store.close().unwrap();
    let refused = store.save_now().unwrap_err();

    assert_eq!(every_way_a_flush_can_fail(refused), "already closed");
}

#[test]
fn a_match_over_every_way_an_open_fails_needs_no_catch_all() {
    let at = TempPath::new("opens_exhaustive");
    std::fs::create_dir_all(at.path()).unwrap();

    let refused = StoreBuilder::new(at.path()).build().unwrap_err();

    assert!(
        every_way_an_open_can_fail(refused).starts_with("would not open"),
        "a directory where the file goes is an open failure"
    );
}

fn through_anyhow(at: &TempPath) -> anyhow::Result<Option<u16>> {
    let store = StoreBuilder::new(at.path()).build()?;
    store.set(["port"], &"not a number".to_string())?;
    store.save_now()?;
    let held = store.get::<u16>(["port"])?;
    store.close()?;
    Ok(held)
}

fn through_a_box(at: &TempPath) -> Result<Option<u16>, Box<dyn Error + Send + Sync>> {
    let store = StoreBuilder::new(at.path()).build()?;
    store.set(["port"], &"not a number".to_string())?;
    let held = store.get::<u16>(["port"])?;
    Ok(held)
}

#[test]
fn an_open_a_write_a_flush_a_read_and_a_close_all_go_into_anyhow() {
    let at = TempPath::new("reads_anyhow");

    let carried = through_anyhow(&at).unwrap_err();

    assert!(
        carried.to_string().contains("port"),
        "anyhow keeps what the refusal said, got: {carried}"
    );
}

#[test]
fn the_same_five_go_into_a_boxed_error() {
    let at = TempPath::new("reads_boxed");

    let carried = through_a_box(&at).unwrap_err();

    assert!(
        carried.to_string().contains("port"),
        "a boxed error keeps it too, got: {carried}"
    );
}
