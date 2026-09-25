use amethystate::store::builder::StoreBuilder;
use amethystate::{StoreBackend, global_store};
use amethystate_core::test_utils::TempPath;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[test]
fn a_global_store_is_closed_by_awaiting_its_guard() {
    let path = TempPath::new("global_closed_by_awaiting");
    let guard = StoreBuilder::new(path.path()).build_global().unwrap();
    global_store().kv().set("port", &8080u16).unwrap();

    futures::executor::block_on(guard.close_async()).unwrap();

    assert!(global_store().is_closed());

    let reopened = StoreBuilder::new(path.path()).build().unwrap();
    assert_eq!(reopened.kv().get::<u16>("port").unwrap(), Some(8080));
}
