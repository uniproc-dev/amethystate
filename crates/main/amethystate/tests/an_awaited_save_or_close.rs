use amethystate::StoreBuilder;
use amethystate::store::StorageError;
use amethystate::store::builder::Backend;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::time::Duration;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(not(target_arch = "wasm32"))]
const CHILD: &str = "AME_AWAITED_SAVE_CHILD";

#[cfg(not(target_arch = "wasm32"))]
fn save_then_abort(backend: Backend, path: &Path) -> ! {
    let store = StoreBuilder::new(path)
        .backend(backend)
        .disk(|d| d.debounce(Duration::from_secs(600)))
        .build()
        .unwrap();

    store.kv().set("port", &8080u16).unwrap();
    futures::executor::block_on(store.save_async()).unwrap();

    std::process::abort();
}

#[cfg(not(target_arch = "wasm32"))]
fn save_crash_then_reopen(test_name: &str, backend: Backend) {
    if let Ok(child_path) = std::env::var(CHILD) {
        save_then_abort(backend, Path::new(&child_path));
    }

    let path = TempPath::new(test_name);

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(test_name)
        .env(CHILD, path.path())
        .output()
        .expect("spawning the writer failed");

    assert!(
        !status.status.success(),
        "the writer was supposed to abort, not exit cleanly"
    );

    let store = StoreBuilder::new(&path)
        .backend(backend)
        .build()
        .expect("the crashed writer left the store openable");

    assert_eq!(
        store.kv().get::<u16>("port").unwrap(),
        Some(8080),
        "{backend:?}: the awaited save had not reached the disk when the process died"
    );
}

macro_rules! saved_across_a_crash {
    ($feature:literal, $name:ident, $backend:expr) => {
        #[cfg(feature = $feature)]
        #[test]
        fn $name() {
            save_crash_then_reopen(stringify!($name), $backend);
        }
    };
}

saved_across_a_crash!(
    "redb",
    an_awaited_save_survives_a_crash_on_redb,
    Backend::Redb
);
saved_across_a_crash!(
    "sqlite",
    an_awaited_save_survives_a_crash_on_sqlite,
    Backend::Sqlite
);
saved_across_a_crash!(
    "json",
    an_awaited_save_survives_a_crash_on_json,
    Backend::Json
);
saved_across_a_crash!(
    "toml",
    an_awaited_save_survives_a_crash_on_toml,
    Backend::Toml
);
saved_across_a_crash!("ron", an_awaited_save_survives_a_crash_on_ron, Backend::Ron);

#[backends(all)]
fn an_awaited_close_writes_what_was_buffered_and_lets_go_of_the_file(backend: Backend) {
    let path = TempPath::new("awaited_close");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .disk(|d| d.debounce(Duration::from_secs(600)))
        .build()
        .unwrap();

    store.kv().set("port", &8080u16).unwrap();
    futures::executor::block_on(store.close_async()).unwrap();

    assert!(store.is_closed(), "{backend:?}");
    let refused = store
        .get_raw(&amethystate::store::StorePath::from_segments(["port"]))
        .unwrap_err();
    assert_eq!(
        *refused.current_context(),
        StorageError::Closed,
        "{backend:?}: {refused:?}"
    );

    futures::executor::block_on(store.close_async())
        .unwrap_or_else(|why| panic!("{backend:?}: a second close answered {why:?}"));

    let reopened = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap_or_else(|why| panic!("{backend:?}: the closed store still held its file: {why}"));
    assert_eq!(reopened.kv().get::<u16>("port").unwrap(), Some(8080));
}

#[test]
fn a_save_awaited_on_a_closed_store_says_it_was_closed() {
    let path = TempPath::new("awaited_save_after_close");
    let store = StoreBuilder::new(path.path()).build().unwrap();
    store.close().unwrap();

    let refused = futures::executor::block_on(store.save_async()).unwrap_err();
    assert_eq!(*refused.current_context(), StorageError::Closed);
}
