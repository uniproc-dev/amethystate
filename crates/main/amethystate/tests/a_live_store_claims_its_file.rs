use amethystate::Store;
use amethystate::store::StorageError;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;

mod common;

fn written(path: &TempPath, backend: Backend) -> Store {
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    store.set(["a"], &1u8).unwrap();
    store.save_now().unwrap();
    store
}

#[allow(unreachable_patterns)]
fn refuses_a_second_store(backend: Backend) -> bool {
    match backend {
        #[cfg(feature = "redb")]
        Backend::Redb => true,
        _ => false,
    }
}

#[allow(unreachable_patterns)]
fn holds_the_file(backend: Backend) -> bool {
    match backend {
        #[cfg(feature = "sqlite")]
        Backend::Sqlite => true,
        _ => false,
    }
}

#[test]
fn only_redb_keeps_a_second_store_from_opening_the_same_file() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("claims_second_open");
        let first = written(&path, backend);

        let second = StoreBuilder::new(path.path()).backend(backend).build();

        assert_eq!(
            second.is_err(),
            refuses_a_second_store(backend),
            "{backend:?} answered a second open with {:?}",
            second.as_ref().map(|_| "a store")
        );

        drop(second);
        drop(first);
    }
}

#[test]
fn a_closed_store_lets_a_second_one_open_the_file() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("claims_closed_second");
        let first = written(&path, backend);
        first.close().unwrap();

        let second = StoreBuilder::new(path.path()).backend(backend).build();

        assert!(
            second.is_ok(),
            "{backend:?} refused a second open after close: {:?}",
            second.err()
        );

        drop(second);
        drop(first);
    }
}

#[cfg(windows)]
#[test]
fn a_closed_store_lets_the_platform_have_the_file() {
    for backend in common::enabled_backends() {
        let removing = TempPath::new("claims_closed_remove");
        let store = written(&removing, backend);
        store.close().unwrap();
        let removed = std::fs::remove_file(removing.path());
        assert!(removed.is_ok(), "{backend:?} still held the file: {removed:?}");
        drop(store);

        let renaming = TempPath::new("claims_closed_rename");
        let beside = renaming.path().with_extension("moved");
        let store = written(&renaming, backend);
        store.close().unwrap();
        let renamed = std::fs::rename(renaming.path(), &beside);
        assert!(renamed.is_ok(), "{backend:?} still held the file: {renamed:?}");
        drop(store);
        let _ = std::fs::remove_file(&beside);
    }
}

#[test]
fn a_write_after_a_close_is_refused_rather_than_lost() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("claims_closed_write");
        let store = written(&path, backend);
        store.close().unwrap();

        let refused = store
            .set(["b"], &2u8)
            .expect_err(&format!("{backend:?} took a write after close"));

        assert_eq!(refused.current_context(), &StorageError::Closed, "{backend:?}");
    }
}

#[test]
fn what_was_written_before_a_close_is_there_afterwards() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("claims_closed_landed");
        let store = written(&path, backend);
        store.close().unwrap();
        drop(store);

        let again = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();

        assert_eq!(again.get::<u8>(["a"]).unwrap(), Some(1), "{backend:?}");
    }
}

#[test]
fn closing_twice_says_the_same_as_closing_once() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("claims_closed_twice");
        let store = written(&path, backend);

        store.close().unwrap();
        store.close().unwrap();
    }
}

#[cfg(windows)]
#[test]
fn only_sqlite_holds_the_file_against_the_platform() {
    for backend in common::enabled_backends() {
        let held = holds_the_file(backend);

        let removing = TempPath::new("claims_remove");
        let store = written(&removing, backend);
        let removed = std::fs::remove_file(removing.path());
        assert_eq!(
            removed.is_err(),
            held,
            "{backend:?} answered a removal with {removed:?}"
        );
        drop(store);

        let renaming = TempPath::new("claims_rename");
        let beside = renaming.path().with_extension("moved");
        let store = written(&renaming, backend);
        let renamed = std::fs::rename(renaming.path(), &beside);
        assert_eq!(
            renamed.is_err(),
            held,
            "{backend:?} answered a rename with {renamed:?}"
        );
        drop(store);
        let _ = std::fs::remove_file(&beside);
    }
}
