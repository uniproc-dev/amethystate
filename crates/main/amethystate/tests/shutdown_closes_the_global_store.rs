use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{IntoGlobalStore, global_store};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

mod common;

#[backends(Redb)]
fn a_write_after_shutdown_is_refused_and_what_came_before_it_is_on_disk(backend: Backend) {
    let path = TempPath::new("global_shutdown");
    let guard = StoreBuilder::new(path.path())
        .backend(backend)
        .init_global();

    global_store().kv().set("port", &8080u16).unwrap();

    amethystate::shutdown().unwrap();

    let refused = global_store()
        .kv()
        .set("port", &9090u16)
        .expect_err("a closed global store took a write");
    insta::assert_snapshot!("write_after_shutdown", refused.to_string());

    assert!(amethystate::shutdown().is_ok());

    drop(guard);

    let reopened = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    assert_eq!(reopened.kv().get::<u16>("port").unwrap(), Some(8080));
}
