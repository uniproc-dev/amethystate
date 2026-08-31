//! Regressions for failures the library used to answer with a plausible value.
//!
//! Each of these was a silent substitution or a dropped `Result` found by an
//! audit rather than by a test, which is how they survived. The store is
//! allowed to fail; it is not allowed to invent.

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::reactive_map_with_path_only;
use amethystate::store::{StorageError, StoreBackend};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::collections::HashMap;
use uuid::Uuid;

/// Bytes that are not the asked-for type are a `Codec` failure.
///
/// They used to be `Ok(T::default())` with a warning, so a caller could not
/// tell a corrupt entry from a real zero.
#[backends(all)]
fn undecodable_bytes_are_an_error_rather_than_a_default(backend: Backend) {
    let path = TempPath::new("decode_strict");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["port"], &"not a number".to_string()).unwrap();
    let raw = StoreBackend::get_raw(&store, &StorePath::from_segments(["port"]))
        .unwrap()
        .expect("the bytes are there");

    let err = store.decode::<u16>(&raw).unwrap_err();

    assert_eq!(err.current_context(), &StorageError::Codec, "got {err:?}");
}

/// A map entry whose stored value is the wrong type is not read back as a
/// default.
///
/// This is the shape the substitution was hiding: the entry answered `Some(0)`
/// to a read and an error to a write, in the same process, one line apart.
#[backends(all)]
fn a_map_entry_of_the_wrong_type_does_not_read_back_as_a_default(backend: Backend) {
    let path = TempPath::new("map_wrong_type");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["cols", "cpu"], &"wide".to_string()).unwrap();
    store.save_now().unwrap();

    let err = reactive_map_with_path_only::<String, u32>(
        &store,
        ["cols"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap_err();

    assert_eq!(err.current_context(), &StorageError::Codec, "got {err:?}");
    assert!(
        format!("{err:?}").contains("cpu"),
        "the report names the entry it could not read: {err:?}"
    );
}

/// A default whose key cannot name a level fails the map rather than being
/// dropped on the way to disk.
#[backends(all)]
fn a_map_default_whose_key_is_empty_fails_rather_than_vanishing(backend: Backend) {
    let path = TempPath::new("map_empty_default");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let defaults = HashMap::from([(String::new(), 1u32)]);
    let err =
        reactive_map_with_path_only::<String, u32>(&store, ["sizes"], defaults, Uuid::new_v4())
            .unwrap_err();

    assert_eq!(err.current_context(), &StorageError::Path, "got {err:?}");
}

/// A path a caller names is checked before anything is written.
#[backends(all)]
fn an_empty_level_is_refused_and_the_store_is_untouched(backend: Backend) {
    let path = TempPath::new("empty_level");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["kept"], &1u32).unwrap();

    let err = store.set(["kept", ""], &2u32).unwrap_err();

    assert_eq!(err.current_context(), &StorageError::Path, "got {err:?}");
    assert_eq!(store.get::<u32>(["kept"]).unwrap(), Some(1));
}
