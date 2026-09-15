use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{OpenStore, StoreLayout, WillNotOpen};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "cache")]
pub struct Cache {
    #[amestate(default = 1u64)]
    pub generation: u64,
}

/// Bytes that are not this store, whichever store it is: not a database, not a
/// document, and not empty - an empty file is a valid empty document to toml.
const RUBBISH: &[u8] = b"\x00\x01not a store at all\x00[[[";

fn written(backend: Backend, at: &TempPath) -> std::path::PathBuf {
    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap();

    let held = Cache::new_with(&store).unwrap();
    held.generation().set(42).unwrap();
    store.save_now().unwrap();
    store.close().unwrap();

    match StoreLayout::of(at.path(), backend) {
        StoreLayout::Single { data } => data,
        StoreLayout::Sidecars { data, .. } => data,
    }
}

#[backends(all)]
fn a_store_that_will_not_open_is_refused_and_left_alone(backend: Backend) {
    let at = TempPath::new("will_not_open_refused");
    let data = written(backend, &at);
    std::fs::write(&data, RUBBISH).unwrap();

    let refused = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .expect_err(&format!(
            "{backend:?} opened over bytes that are not a store"
        ));

    assert!(
        matches!(refused, OpenStore::WouldNotOpen { .. }),
        "{backend:?} said {refused}"
    );

    assert_eq!(
        std::fs::read(&data).unwrap(),
        RUBBISH,
        "{backend:?}: the file was touched by an open that was refused"
    );
}

#[backends(all)]
fn a_store_told_to_start_fresh_opens_empty_over_what_it_could_not_read(backend: Backend) {
    let at = TempPath::new("will_not_open_fresh");
    let data = written(backend, &at);
    std::fs::write(&data, RUBBISH).unwrap();

    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .when_it_will_not_open(WillNotOpen::StartFresh)
        .build()
        .unwrap_or_else(|why| panic!("{backend:?} refused after being told to start fresh: {why}"));

    assert_eq!(
        store
            .get::<u64>(StorePath::from_segments(["cache", "generation"]))
            .unwrap(),
        None,
        "{backend:?}: the store that opened is holding what the old one did"
    );

    let held = Cache::new_with(&store).unwrap();
    assert_eq!(
        held.generation().get(),
        1,
        "{backend:?}: the declared default did not seed the fresh store"
    );

    held.generation().set(7).unwrap();
    store.save_now().unwrap();
    store.close().unwrap();

    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap_or_else(|why| panic!("{backend:?}: the fresh store did not reopen: {why}"));

    assert_eq!(
        Cache::new_with(&store).unwrap().generation().get(),
        7,
        "{backend:?}: what the fresh store wrote did not survive"
    );
}

#[backends(all)]
fn starting_fresh_is_about_the_files_and_not_about_the_directory(backend: Backend) {
    let at = TempPath::new("will_not_open_directory");
    let inside = at.path().join("under").join("a").join("file");
    std::fs::create_dir_all(at.path()).unwrap();
    std::fs::write(at.path().join("under"), b"not a directory").unwrap();

    let refused = StoreBuilder::new(&inside)
        .backend(backend)
        .when_it_will_not_open(WillNotOpen::StartFresh)
        .build()
        .expect_err(&format!(
            "{backend:?} opened a store under a path that is a file"
        ));

    assert!(
        matches!(
            refused,
            OpenStore::WouldNotOpen { .. } | OpenStore::Store(_)
        ),
        "{backend:?} said {refused}"
    );
}
