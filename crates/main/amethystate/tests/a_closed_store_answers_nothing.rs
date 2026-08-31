use amethystate::store::StorageError;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;

mod common;

#[test]
fn a_closed_store_answers_every_engine_the_same_way() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("closed_reads");
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        store.set(["a"], &1u8).unwrap();
        store.close().unwrap();

        let read = store
            .get::<u8>(["a"])
            .expect_err(&format!("{backend:?} read from a closed store"));
        assert_eq!(read.current_context(), &StorageError::Closed, "{backend:?}");

        let scanned = store
            .scan_keys(&StorePath::root())
            .expect_err(&format!("{backend:?} scanned a closed store"));
        assert_eq!(
            scanned.current_context(),
            &StorageError::Closed,
            "{backend:?}"
        );

        let removed = store
            .delete(["a"])
            .expect_err(&format!("{backend:?} deleted from a closed store"));
        assert_eq!(
            removed.current_context(),
            &StorageError::Closed,
            "{backend:?}"
        );
    }
}
