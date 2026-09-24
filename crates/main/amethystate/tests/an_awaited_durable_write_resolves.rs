use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::time::Duration;

#[amethystate(prefix = "awaited")]
pub struct Awaited {
    #[amestate(default = 1)]
    pub port: u16,
}

#[cfg(not(target_arch = "wasm32"))]
fn resolved_within<T: Send + 'static>(what: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(what());
    });

    rx.recv_timeout(Duration::from_secs(10)).ok()
}

#[cfg(target_arch = "wasm32")]
fn resolved_within<T>(what: impl FnOnce() -> T) -> Option<T> {
    Some(what())
}

#[backends(all)]
fn an_awaited_durable_write_resolves_once_it_is_on_disk(backend: Backend) {
    let at = TempPath::new("awaited_durable");
    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .disk(|d| d.debounce(Duration::from_secs(60)))
        .build()
        .unwrap();
    let state = Awaited::new_with(&store).unwrap();

    let written = resolved_within(move || {
        futures::executor::block_on(state.port().durable().set_async(8080))
    });

    assert!(matches!(written, Some(Ok(()))), "{backend:?}: {written:?}");
}

#[backends(all)]
fn an_awaited_durable_write_with_nothing_left_to_flush_resolves_too(backend: Backend) {
    let at = TempPath::new("awaited_durable_nothing");
    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .disk(|d| d.debounce(Duration::from_secs(60)))
        .build()
        .unwrap();
    let state = Awaited::new_with(&store).unwrap();
    store.save_now().unwrap();

    let written =
        resolved_within(move || futures::executor::block_on(state.port().durable().set_async(1)));

    assert!(matches!(written, Some(Ok(()))), "{backend:?}: {written:?}");
}
