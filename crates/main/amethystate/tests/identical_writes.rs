use amethystate::amethystate;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::test_utils::TempPath;
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;

#[amethystate(prefix = "net")]
pub struct ConnectionState {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

#[test]
fn writing_the_same_value_wakes_nobody() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("identical_field");
    let store = StoreBuilder::new(path.path()).build()?;
    let state = ConnectionState::new_with(&store)?;

    state.port().set(9090)?;

    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let _sub = state.port().subscribe(move |_| {
        count.fetch_add(1, Ordering::Release);
    });

    state.port().set(9090)?;
    state.port().set(9090)?;
    state.port().set(9090)?;

    assert_eq!(woken.load(Ordering::Acquire), 0);
    assert_eq!(state.port().get(), 9090);

    Ok(())
}

#[test]
fn a_different_value_still_arrives() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("identical_then_different");
    let store = StoreBuilder::new(path.path()).build()?;
    let state = ConnectionState::new_with(&store)?;

    state.port().set(9090)?;

    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let _sub = state.port().subscribe(move |_| {
        count.fetch_add(1, Ordering::Release);
    });

    state.port().set(9090)?;
    state.port().set(1234)?;
    state.port().set(1234)?;

    assert_eq!(woken.load(Ordering::Acquire), 1);
    assert_eq!(state.port().get(), 1234);

    Ok(())
}

#[test]
fn the_store_itself_deduplicates() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("identical_store");
    let store = StoreBuilder::new(path.path()).build()?;

    store.set(["raw", "value"], &42u32)?;
    store.save_now()?;

    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let _sub = store.subscribe(
        amethystate::SubscriptionKind::ExactPath(amethystate_core::path::StorePath::from_segments(
            ["raw", "value"],
        )),
        Arc::new(move |_| {
            count.fetch_add(1, Ordering::Release);
        }),
    );

    store.set(["raw", "value"], &42u32)?;

    assert_eq!(woken.load(Ordering::Acquire), 0);

    store.set(["raw", "value"], &43u32)?;

    assert_eq!(woken.load(Ordering::Acquire), 1);

    Ok(())
}

#[test]
fn a_committed_value_deduplicates_too() -> Result<(), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new("identical_after_flush");
    let store = StoreBuilder::new(path.path()).build()?;
    let state = ConnectionState::new_with(&store)?;

    state.port().durable().set(9090)?;

    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let _sub = state.port().subscribe(move |_| {
        count.fetch_add(1, Ordering::Release);
    });

    state.port().set(9090)?;

    assert_eq!(woken.load(Ordering::Acquire), 0);

    Ok(())
}

#[cfg(any(feature = "json", feature = "toml", feature = "ron"))]
#[test]
fn every_text_engine_deduplicates() -> Result<(), Box<dyn Error + Send + Sync>> {
    for backend in common::text_backends() {
        let path = TempPath::new(&format!("identical_{}", backend.extension()));
        let store = StoreBuilder::new(path.path()).backend(backend).build()?;
        let state = ConnectionState::new_with(&store)?;

        state.port().set(9090)?;

        let woken = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&woken);
        let _sub = state.port().subscribe(move |_| {
            count.fetch_add(1, Ordering::Release);
        });

        state.port().set(9090)?;

        assert_eq!(
            woken.load(Ordering::Acquire),
            0,
            "on {}",
            backend.extension()
        );
    }

    Ok(())
}
