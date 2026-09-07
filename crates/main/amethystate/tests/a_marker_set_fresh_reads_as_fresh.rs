use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{InitState, StoreBackend, StorePath};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[backends(all)]
fn a_namespace_just_set_fresh_does_not_read_as_seeded(backend: Backend) {
    let path = TempPath::new("marker_fresh");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let namespace = StorePath::segment("things");

    store
        .set_initialized(&namespace, InitState::Seeded)
        .unwrap();
    store.save_now().unwrap();
    assert!(store.is_initialized(&namespace).unwrap());

    store.set_initialized(&namespace, InitState::Fresh).unwrap();

    assert!(
        !store.is_initialized(&namespace).unwrap(),
        "the marker was cleared and the answer still comes off the disk, on {}",
        backend.extension()
    );
}
