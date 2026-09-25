use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::time::Duration;

mod common;

#[amethystate(prefix = "after_close")]
pub struct Settings {
    #[amestate(default = 1)]
    pub port: u16,
}

#[cfg(not(target_arch = "wasm32"))]
fn answered_within<T: Send + 'static>(what: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(what());
    });

    rx.recv_timeout(Duration::from_secs(10))
        .expect("a durable write after a close never answered")
}

#[cfg(target_arch = "wasm32")]
fn answered_within<T>(what: impl FnOnce() -> T) -> T {
    what()
}

#[backends(all)]
fn a_durable_write_after_a_close_is_refused_rather_than_awaited(backend: Backend) {
    let path = TempPath::new("durable_after_close");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .disk(|d| d.debounce(Duration::from_secs(600)))
        .build()
        .unwrap();
    let state = Settings::new_with(&store).unwrap();

    store.close().unwrap();

    let refused = answered_within(move || state.port().durable().set(8080))
        .expect_err("a durable write was taken by a closed store");

    insta::assert_snapshot!("durable_set_after_close", refused.to_string());
}

#[backends(all)]
fn a_flush_awaited_on_a_closed_store_is_answered(backend: Backend) {
    let path = TempPath::new("flush_async_after_close");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .disk(|d| d.debounce(Duration::from_secs(600)))
        .build()
        .unwrap();

    store.close().unwrap();

    let refused = answered_within(move || futures::executor::block_on(store.flush_async()))
        .expect_err("a flush was awaited on a closed store and answered as landed");

    assert_eq!(
        *refused.current_context(),
        amethystate::store::StorageError::Closed,
        "{backend:?}: {refused:?}"
    );
}

#[backends(all)]
fn an_awaited_durable_write_after_a_close_is_refused_too(backend: Backend) {
    let path = TempPath::new("durable_async_after_close");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .disk(|d| d.debounce(Duration::from_secs(600)))
        .build()
        .unwrap();
    let state = Settings::new_with(&store).unwrap();

    store.close().unwrap();

    let refused = answered_within(move || {
        futures::executor::block_on(state.port().durable().set_async(8080))
    })
    .expect_err("an awaited durable write was taken by a closed store");

    insta::assert_snapshot!("durable_set_async_after_close", refused.to_string());
}
