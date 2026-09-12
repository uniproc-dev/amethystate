//! Regressions for failures the library used to answer with a plausible value.
//!
//! Each of these was a silent substitution or a dropped `Result` found by an
//! audit rather than by a test, which is how they survived. The store is
//! allowed to fail; it is not allowed to invent.

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::reactive_map_with_path_only;
use amethystate::store::{LoadMap, StorageError, StoreBackend};
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

    let LoadMap::EntryWillNotRead { at, why } = err else {
        panic!("{err}")
    };

    assert_eq!(at.to_string(), "cols.cpu");
    assert_eq!(why.current_context(), &StorageError::Codec, "got {why:?}");
}

/// A default whose key is the empty name reaches the disk rather than being
/// dropped on the way there.
#[backends(all)]
fn a_map_default_whose_key_is_empty_reaches_the_disk(backend: Backend) {
    let path = TempPath::new("map_empty_default");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let defaults = HashMap::from([(String::new(), 1u32)]);
    let sizes =
        reactive_map_with_path_only::<String, u32>(&store, ["sizes"], defaults, Uuid::new_v4())
            .unwrap();

    assert_eq!(sizes.get(""), Some(1));
    assert_eq!(sizes.keys().collect::<Vec<_>>(), [""]);

    store.save_now().unwrap();
    drop(sizes);
    drop(store);

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let sizes = reactive_map_with_path_only::<String, u32>(
        &store,
        ["sizes"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap();

    assert_eq!(
        sizes.get(""),
        Some(1),
        "the seeded default did not come back"
    );
}
