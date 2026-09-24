#![cfg(feature = "memory")]

use amethystate::StoreBackend;
use amethystate::amethystate;
use amethystate::migration::MigrationError;
use amethystate::store::builder::{Backend, StoreBuilder, WithSteps};
use amethystate::store::{OpenStore, Persistence, StorageError, StoreLayout};
#[cfg(feature = "redb")]
use amethystate::{StoreOp, SubscriptionKind};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
#[cfg(feature = "redb")]
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[amethystate(prefix = "cache")]
pub struct Cache {
    #[amestate(default = 1u64)]
    pub generation: u64,
}

const RUBBISH: &[u8] = b"\x00\x01not a store at all\x00[[[";

fn data_file(backend: Backend, at: &TempPath) -> std::path::PathBuf {
    match StoreLayout::of(at.path(), backend) {
        StoreLayout::Single { data } => data,
        StoreLayout::Sidecars { data, .. } => data,
        other => panic!("{backend:?} keeps no file: {other:?}"),
    }
}

#[backends(all)]
fn a_store_that_opens_says_it_is_on_disk(backend: Backend) {
    let at = TempPath::new("fallback_on_disk");

    let (store, persistence) = StoreBuilder::new(at.path())
        .backend(backend)
        .or_in_memory()
        .build();

    assert!(
        matches!(persistence, Persistence::OnDisk),
        "{backend:?}: {persistence:?}"
    );
    assert_ne!(
        store.files_layout(),
        Some(StoreLayout::InMemory),
        "{backend:?}"
    );
}

#[backends(files)]
fn a_file_that_will_not_open_leaves_a_working_store_in_memory_and_is_left_alone(backend: Backend) {
    let at = TempPath::new("fallback_rubbish");
    let data = data_file(backend, &at);
    std::fs::write(&data, RUBBISH).unwrap();

    let (store, persistence) = StoreBuilder::new(at.path())
        .backend(backend)
        .or_in_memory()
        .build();

    let Persistence::InMemory { because } = &persistence else {
        panic!("{backend:?} opened over bytes that are not a store: {persistence:?}");
    };
    assert!(
        matches!(because, OpenStore::WouldNotOpen { .. }),
        "{backend:?} fell back for another reason: {because}"
    );
    assert_eq!(store.files_layout(), Some(StoreLayout::InMemory));

    let held = Cache::new_with(&store).unwrap();
    assert_eq!(
        held.generation().get(),
        1,
        "{backend:?}: defaults were not seeded"
    );
    held.generation().set(7).unwrap();
    assert_eq!(held.generation().get(), 7);

    store.save_now().unwrap();
    store.close().unwrap();
    drop(held);
    drop(store);

    assert_eq!(
        std::fs::read(&data).unwrap(),
        RUBBISH,
        "{backend:?}: the store in memory touched the file it stood in for"
    );
}

#[backends(Redb)]
fn a_file_another_store_holds_is_answered_in_memory(backend: Backend) {
    let at = TempPath::new("fallback_held");
    let holder = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap();
    holder.kv().set("port", &8080u16).unwrap();
    holder.save_now().unwrap();

    let (second, persistence) = StoreBuilder::new(at.path())
        .backend(backend)
        .or_in_memory()
        .build();

    assert!(
        persistence.is_in_memory(),
        "a second store opened a file the first one holds: {persistence:?}"
    );
    assert_eq!(second.kv().get::<u16>("port").unwrap(), None);

    second.kv().set("port", &9090u16).unwrap();
    assert_eq!(holder.kv().get::<u16>("port").unwrap(), Some(8080));
}

fn to_version(backend: Backend, at: &TempPath, version: u32) -> WithSteps {
    StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(move |m| {
            m.for_prefix("app").step(version, "up", |_| Ok(()));
        })
}

#[backends(all)]
fn a_file_a_newer_release_wrote_is_kept_for_that_release(backend: Backend) {
    let at = TempPath::new("fallback_downgrade");

    let (newer, _) = to_version(backend, &at, 2).migrate().unwrap();
    newer
        .set(StorePath::from_segments(["app", "theme"]), &"dark")
        .unwrap();
    newer.save_now().unwrap();
    newer.close().unwrap();
    drop(newer);

    let (older, report, persistence) = to_version(backend, &at, 1).or_in_memory().migrate();
    assert!(report.components.is_empty(), "{backend:?}: {report:?}");

    let Some(OpenStore::Migrating {
        report: Some(refused),
        ..
    }) = persistence.because()
    else {
        panic!("{backend:?}: an older release opened a newer file: {persistence:?}");
    };
    let failures = refused.failures().collect::<Vec<_>>();
    assert!(
        matches!(
            failures.as_slice(),
            [only] if matches!(
                only.downcast_ref::<MigrationError>(),
                Some(MigrationError::Downgrade {
                    db_version: 2,
                    code_version: 1,
                    ..
                })
            )
        ),
        "{backend:?}: {failures:?}"
    );

    older
        .set(StorePath::from_segments(["app", "theme"]), &"light")
        .unwrap();
    older.close().unwrap();
    drop(older);

    let (newer, _) = to_version(backend, &at, 2)
        .migrate()
        .unwrap_or_else(|why| panic!("{backend:?}: the newer release lost its file: {why}"));
    assert_eq!(
        newer
            .get::<String>(StorePath::from_segments(["app", "theme"]))
            .unwrap()
            .as_deref(),
        Some("dark"),
        "{backend:?}"
    );
}

#[test]
fn a_store_asked_for_in_memory_writes_no_file() {
    let at = TempPath::new("memory_no_file");

    let store = StoreBuilder::new(at.path())
        .backend(Backend::Memory)
        .build()
        .unwrap();
    store.kv().set("port", &8080u16).unwrap();
    store.save_now().unwrap();
    store.close().unwrap();

    assert!(
        !at.path().exists() && !at.path().with_extension("").exists(),
        "a store in memory left a file behind"
    );
}

#[test]
fn a_closed_store_in_memory_answers_closed() {
    let store = StoreBuilder::in_memory().build().unwrap();
    store.kv().set("port", &8080u16).unwrap();
    store.close().unwrap();

    let refused = store
        .get_raw(&StorePath::from_segments(["port"]))
        .unwrap_err();
    assert_eq!(*refused.current_context(), StorageError::Closed);
    assert!(store.is_closed());
}

#[test]
fn a_durable_write_in_memory_does_not_wait() {
    let store = StoreBuilder::in_memory().build().unwrap();
    store.kv().set("port", &8080u16).unwrap();

    futures::executor::block_on(store.flush_async()).unwrap();
}

#[cfg(feature = "redb")]
type Event = (StoreOp, String, Option<u16>, Option<u16>);
#[cfg(feature = "redb")]
type Heard = Arc<Mutex<Vec<Event>>>;

#[cfg(feature = "redb")]
fn listened(store: &amethystate::Store) -> Heard {
    let heard: Heard = Arc::default();
    let keep = heard.clone();
    let decoding = store.clone();

    StoreBackend::subscribe(
        store,
        SubscriptionKind::Any,
        Arc::new(move |event| {
            let read = |bytes: &Option<Vec<u8>>| {
                bytes
                    .as_ref()
                    .and_then(|bytes| decoding.decode::<u16>(bytes).ok())
            };
            keep.lock().unwrap().push((
                event.op,
                event.path.to_string(),
                read(&event.old),
                read(&event.new),
            ));
            Ok(())
        }),
    );

    heard
}

#[cfg(feature = "redb")]
fn walked(store: &amethystate::Store) -> (Vec<Event>, Vec<String>) {
    let heard = listened(store);
    let at = |levels: &[&str]| StorePath::from_segments(levels.iter().copied());

    store.set(at(&["ui", "width"]), &240u16).unwrap();
    store.set(at(&["ui", "width"]), &240u16).unwrap();
    store.set(at(&["ui", "width"]), &320u16).unwrap();
    store.set(at(&["ui", "panel", "height"]), &80u16).unwrap();
    store.set(at(&["uix"]), &1u16).unwrap();
    store.delete(at(&["missing"])).unwrap();
    store.delete(at(&["uix"])).unwrap();

    let keys = store
        .scan_keys(at(&["ui"]))
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    store.delete_prefix(at(&["ui"])).unwrap();
    assert!(store.scan_prefix(at(&["ui"])).unwrap().is_empty());

    let heard = heard.lock().unwrap().clone();
    (heard, keys)
}

#[cfg(feature = "redb")]
#[test]
fn a_store_in_memory_answers_as_redb_does() {
    let at = TempPath::new("memory_as_redb");
    let redb = StoreBuilder::new(at.path())
        .backend(Backend::Redb)
        .build()
        .unwrap();
    let memory = StoreBuilder::in_memory().build().unwrap();

    let on_disk = walked(&redb);
    let in_memory = walked(&memory);

    assert_eq!(in_memory, on_disk);
    insta::assert_debug_snapshot!(in_memory);
}
