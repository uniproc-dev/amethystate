use amethystate::StoreBackend;
use amethystate::prelude::{OnDelete, OnUnreadable};
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::field_with_path_where;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use uuid::Uuid;

#[backends(all)]
fn a_delete_that_restores_the_default_clears_the_disagreement(backend: Backend) {
    let path = TempPath::new("delete_settles");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let port = field_with_path_where::<u64>(
        &store,
        ["net", "port"],
        8080,
        Uuid::new_v4(),
        OnUnreadable::UseDefault,
        OnDelete::UseDefault,
    )
    .unwrap();

    store.set(["net", "port"], &"not a number").unwrap();
    assert!(
        port.try_get().is_err(),
        "the store holds something the field cannot read, on {}",
        backend.extension()
    );

    StoreBackend::delete(&store, &StorePath::from_segments(["net", "port"])).unwrap();

    assert_eq!(port.get(), 8080, "on {}", backend.extension());
    assert_eq!(
        port.try_get().ok(),
        Some(8080),
        "the value it disagreed with is gone, so the disagreement has to go with it, on {}",
        backend.extension()
    );
}
