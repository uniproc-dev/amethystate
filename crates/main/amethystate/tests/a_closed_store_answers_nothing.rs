use amethystate::store::builder::StoreBuilder;
use amethystate::store::{ReadValue, ScanKeys, WriteValue};
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
        assert!(
            matches!(read, ReadValue::Closed { .. }),
            "{backend:?}: {read}"
        );

        let scanned = store
            .scan_keys(StorePath::root())
            .expect_err(&format!("{backend:?} scanned a closed store"));
        assert!(
            matches!(scanned, ScanKeys::Closed { .. }),
            "{backend:?}: {scanned}"
        );

        let removed = store
            .delete(["a"])
            .expect_err(&format!("{backend:?} deleted from a closed store"));
        assert!(
            matches!(removed, WriteValue::Closed { .. }),
            "{backend:?}: {removed}"
        );

        let swept = store
            .delete_prefix(StorePath::root())
            .expect_err(&format!("{backend:?} swept a closed store"));
        assert!(
            matches!(swept, WriteValue::Closed { .. }),
            "{backend:?}: {swept}"
        );
    }
}
