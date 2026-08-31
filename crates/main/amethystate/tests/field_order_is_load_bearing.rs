//! Whether a struct on disk knows which field is which.
//!
//! The schema snapshot beside the data names every field, and the data did not:
//! the binary codec wrote a struct as an array, so which slot held which name
//! was nowhere in the file. The two halves of the store disagreed about what a
//! struct is, and only one of them was consulted on a read.

#![cfg(feature = "redb")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::field_with_path;
use amethystate::uuid::Uuid;
use amethystate_core::test_utils::TempPath;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct SizeV1 {
    width: u32,
    height: u32,
}

/// The same fields, the same types, declared the other way round. A rename
/// would be the same story with one name instead of two.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct SizeV2 {
    height: u32,
    width: u32,
}

/// Reordering two fields of the same type must not change what the stored
/// values mean.
///
/// Nothing about this is a migration: no version was bumped, because from the
/// author's side nothing about the data changed. When the read was positional
/// the values swapped and every later read was wrong in a way no error
/// reported - 1280 by 720 came back 720 by 1280.
#[test]
fn reordering_two_fields_does_not_swap_what_they_hold() {
    let path = TempPath::new("field_order");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Redb)
            .build()
            .unwrap();
        let size =
            field_with_path::<SizeV1>(&store, ["win", "size"], SizeV1::default(), Uuid::new_v4())
                .unwrap();
        size.set(SizeV1 {
            width: 1280,
            height: 720,
        })
        .unwrap();
        store.save_now().unwrap();
    }

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Redb)
        .build()
        .unwrap();
    let size =
        field_with_path::<SizeV2>(&store, ["win", "size"], SizeV2::default(), Uuid::new_v4())
            .unwrap();

    let read = size.get();
    assert_eq!(
        read,
        SizeV2 {
            width: 1280,
            height: 720
        },
        "a window 1280 wide and 720 high came back {}x{}",
        read.width,
        read.height
    );
}
