use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

fn ns(joined: &str) -> StorePath {
    StorePath::parse_joined(joined).expect("a namespace the test wrote itself")
}

fn opened(backend: Backend, at: &TempPath) -> amethystate::Store {
    StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .expect("the store should open")
}

#[backends(all)]
fn a_marker_written_before_a_value_at_its_name_keeps_both(backend: Backend) {
    let file = TempPath::new("marker_then_value");
    {
        let store = opened(backend, &file);
        store.mark_initialized(&ns("cfg")).unwrap();
        store.set(["cfg"], &7u32).unwrap();
        store.save_now().unwrap();
        store.close().unwrap();
    }

    let store = opened(backend, &file);
    assert_eq!(
        store.get::<u32>(["cfg"]).unwrap(),
        Some(7),
        "the value at `cfg` was lost to the marker for the namespace of the same name"
    );
    assert!(
        store.is_initialized(&ns("cfg")).unwrap(),
        "the marker for `cfg` was lost to the value at the same path"
    );
}

#[backends(all)]
fn a_value_written_before_a_marker_at_its_name_keeps_both(backend: Backend) {
    let file = TempPath::new("value_then_marker");
    {
        let store = opened(backend, &file);
        store.set(["cfg"], &7u32).unwrap();
        store.mark_initialized(&ns("cfg")).unwrap();
        store.save_now().unwrap();
        store.close().unwrap();
    }

    let store = opened(backend, &file);
    assert_eq!(store.get::<u32>(["cfg"]).unwrap(), Some(7));
    assert!(store.is_initialized(&ns("cfg")).unwrap());
}

#[backends(all)]
fn a_marker_and_a_value_are_both_answered_before_anything_is_flushed(backend: Backend) {
    let file = TempPath::new("marker_unflushed");
    let store = opened(backend, &file);

    store.set(["cfg"], &7u32).unwrap();
    store.mark_initialized(&ns("cfg")).unwrap();

    assert_eq!(
        store.get::<u32>(["cfg"]).unwrap(),
        Some(7),
        "a read goes through the buffer, so a marker beside the value must not hide it"
    );
    assert!(store.is_initialized(&ns("cfg")).unwrap());
}
