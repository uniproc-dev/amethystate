#![cfg_attr(not(feature = "bench-internals"), allow(unused_imports))]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

mod common;

/// Declared exactly where the marker for the namespace `foo` is kept, which is
/// the collision: the kind used to be a level on the marker's key and nothing
/// at all on this one's.
#[amethystate(prefix = "init.foo", version = 1)]
pub struct AtTheMarkersPlace {
    #[amestate(default = 7u32)]
    pub width: u32,
}

fn ns(joined: &str) -> StorePath {
    StorePath::parse_joined(joined).expect("a namespace the test wrote itself")
}

/// Opened with a step for `init.foo`, so the engine actually records what that
/// prefix has reached - which is the row the marker used to land on.
fn opened(backend: Backend, at: &TempPath) -> amethystate::Store {
    StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.for_node::<AtTheMarkersPlace>()
                .step(1, "there from the start", |_| Ok(()));
        })
        .build()
        .expect("the store should open")
}

#[backends(all)]
fn a_declared_prefix_does_not_land_on_a_namespaces_marker(backend: Backend) {
    let file = TempPath::new("prefix_over_marker");

    {
        let store = opened(backend, &file);
        store.mark_initialized(&ns("foo")).unwrap();

        let held = AtTheMarkersPlace::new_with(&store).unwrap();
        held.width().set(41).unwrap();

        store.save_now().unwrap();
        store.close().unwrap();
    }

    let store = opened(backend, &file);
    assert!(
        store.is_initialized(&ns("foo")).unwrap(),
        "building a struct declared at `init.foo` wrote its bookkeeping over the marker \
         saying the namespace `foo` had been seeded"
    );

    let held = AtTheMarkersPlace::new_with(&store).unwrap();
    assert_eq!(
        held.width().get(),
        41,
        "the marker for `foo` was written where this struct's bookkeeping lives, so what \
         it recorded about itself came back as something else"
    );
}

/// The bookkeeping table holds three kinds of row in one key space, so what
/// keeps them apart is that no two kinds can ever produce the same key.
///
/// Stated over the keys themselves rather than staged as a collision: the ways
/// a row of one kind can be reached are the ways any name can be written, and a
/// scenario test can only ever try a few of them.
#[cfg(all(feature = "bench-internals", any(feature = "redb", feature = "sqlite")))]
#[test]
fn no_two_kinds_of_bookkeeping_can_land_on_one_key() {
    use amethystate::store::backend::utils::{init_key, prefix_meta_key};

    let adversarial = [
        "foo",
        "init.foo",
        "meta.foo",
        "init",
        "meta",
        "ui.panels",
        "init.ui.panels",
    ];

    for spelled in adversarial {
        let path = StorePath::parse_joined(spelled).expect("a path the test wrote itself");

        assert_ne!(
            init_key(&path).as_bytes(),
            prefix_meta_key(&path).as_bytes(),
            "`{spelled}` reaches the same row whether it is a namespace or a prefix"
        );

        for other in adversarial {
            let them = StorePath::parse_joined(other).expect("a path the test wrote itself");

            assert_ne!(
                init_key(&path).as_bytes(),
                prefix_meta_key(&them).as_bytes(),
                "the marker for the namespace `{spelled}` lands where what the prefix \
                 `{other}` has reached is kept"
            );
        }
    }
}

#[backends(all)]
fn a_marker_for_a_nested_namespace_is_its_own(backend: Backend) {
    let file = TempPath::new("nested_marker");

    {
        let store = opened(backend, &file);
        store.mark_initialized(&ns("ui.panels")).unwrap();
        store.save_now().unwrap();
        store.close().unwrap();
    }

    let store = opened(backend, &file);
    assert!(store.is_initialized(&ns("ui.panels")).unwrap());
    assert!(
        !store.is_initialized(&ns("ui")).unwrap(),
        "marking a namespace marked the one above it"
    );
}
