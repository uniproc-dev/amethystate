#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::store::builder::StoreBuilder;
use amethystate::{Store, amethystate};
use amethystate_core::test_utils::TempPath;
use std::error::Error;
use std::time::Duration;

mod common;

#[amethystate(prefix = "net")]
pub struct ConnectionState {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,
}

fn open(tag: &str) -> Result<(Store, TempPath), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new(tag);
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .build()?;
    Ok((store, path))
}

fn on_disk(path: &TempPath) -> String {
    std::fs::read_to_string(path.path()).unwrap_or_default()
}

#[test]
fn a_write_is_readable_before_it_is_stored() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, path) = open("book_dur_buffered")?;
    let state = ConnectionState::new_with(&store)?;

    //@show a write you can read and the disk cannot
    state.port().set(9090)?;

    let reads_back = state.port().get();
    //@show-end

    assert_eq!(reads_back, 9090);
    assert!(!on_disk(&path).contains("9090"));

    Ok(())
}

#[test]
fn save_now_puts_everything_on_disk() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, path) = open("book_dur_save_now")?;
    let state = ConnectionState::new_with(&store)?;

    //@show forcing everything out
    state.port().set(9090)?;
    store.save_now()?;
    //@show-end

    assert!(on_disk(&path).contains("9090"));

    Ok(())
}

#[test]
fn a_durable_write_returns_after_the_disk() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, path) = open("book_dur_durable")?;
    let state = ConnectionState::new_with(&store)?;

    //@show a write that waits for the disk
    state.port().durable().set(9090)?;
    //@show-end

    assert!(on_disk(&path).contains("9090"));

    Ok(())
}

#[test]
fn a_durable_write_takes_its_neighbours_with_it() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, path) = open("book_dur_neighbours")?;
    let state = ConnectionState::new_with(&store)?;

    //@show what else a durable write commits
    state.host().set("10.0.0.1".to_string())?;

    state.port().durable().set(9090)?;
    //@show-end

    let document = on_disk(&path);

    assert!(document.contains("9090"));
    assert!(document.contains("10.0.0.1"));

    Ok(())
}

#[test]
fn the_window_is_set_at_the_builder() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_dur_window");

    //@show narrowing the window
    let store = StoreBuilder::new(path.path())
        .disk(|d| d.debounce(Duration::from_millis(50)))
        .build()?;
    //@show-end

    store.set(["net", "port"], &9090u16)?;
    store.save_now()?;

    assert_eq!(store.get::<u16>(["net", "port"])?, Some(9090));

    Ok(())
}

#[test]
fn dropping_the_store_flushes_it() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("book_dur_drop");

    {
        let store = StoreBuilder::new(path.path())
            .backend(common::text_backend())
            .build()?;
        let state = ConnectionState::new_with(&store)?;
        state.port().set(9090)?;
    }

    assert!(on_disk(&path).contains("9090"));

    Ok(())
}
