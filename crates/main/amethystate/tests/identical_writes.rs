use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[amethystate(prefix = "net")]
pub struct ConnectionState {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

#[backends(all)]
fn writing_the_same_value_wakes_nobody(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("identical_field");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
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

    assert_eq!(
        woken.load(Ordering::Acquire),
        0,
        "on {}",
        backend.extension()
    );
    assert_eq!(state.port().get(), 9090);

    Ok(())
}

#[backends(all)]
fn a_different_value_still_arrives(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("identical_changed");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    let state = ConnectionState::new_with(&store)?;

    state.port().set(9090)?;

    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let _sub = state.port().subscribe(move |_| {
        count.fetch_add(1, Ordering::Release);
    });

    state.port().set(9091)?;

    assert_eq!(
        woken.load(Ordering::Acquire),
        1,
        "on {}",
        backend.extension()
    );
    assert_eq!(state.port().get(), 9091);

    Ok(())
}

#[backends(all)]
fn the_store_itself_deduplicates(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("identical_store");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;

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
            Ok(())
        }),
    );

    store.set(["raw", "value"], &42u32)?;

    assert_eq!(
        woken.load(Ordering::Acquire),
        0,
        "on {}",
        backend.extension()
    );

    store.set(["raw", "value"], &43u32)?;

    assert_eq!(
        woken.load(Ordering::Acquire),
        1,
        "on {}",
        backend.extension()
    );

    Ok(())
}

#[backends(all)]
fn a_committed_value_deduplicates_too(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("identical_after_flush");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    let state = ConnectionState::new_with(&store)?;

    state.port().durable().set(9090)?;

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

    Ok(())
}
