#[cfg(feature = "ron")]
use amethystate::store::StorageError;
use amethystate::store::builder::StoreBuilder;
#[cfg(feature = "ron")]
use amethystate::store::builder::Backend;
use amethystate_core::test_utils::TempPath;
use serde::{Deserialize, Serialize};

mod common;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
enum Mode {
    Off,
    On(u8),
    Named { level: u8 },
}

fn shapes() -> [(&'static str, Mode); 3] {
    [
        ("unit variant", Mode::Off),
        ("tuple variant", Mode::On(3)),
        ("struct variant", Mode::Named { level: 7 }),
    ]
}

#[test]
fn an_enum_survives_a_round_trip() {
    for backend in common::enabled_backends() {
        if !backend.holds_enums() {
            continue;
        }

        for (label, value) in shapes() {
            let path = TempPath::new("enum_round_trip");
            let store = StoreBuilder::new(path.path())
                .backend(backend)
                .build()
                .unwrap();

            store.set(["probe", "mode"], &value).unwrap();

            let read = store
                .get::<Mode>(["probe", "mode"])
                .unwrap_or_else(|e| panic!("{backend:?} {label}: {e:?}"));

            assert_eq!(read, Some(value), "{backend:?} {label}");
        }
    }
}

#[cfg(feature = "ron")]
#[test]
fn ron_refuses_an_enum_rather_than_dropping_its_variant() {
    for (label, value) in shapes() {
        let path = TempPath::new("enum_refused");
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Ron)
            .build()
            .unwrap();

        let refused = store
            .set(["probe", "mode"], &value)
            .expect_err(&format!("ron took a {label} it cannot read back"));
        assert_eq!(refused.current_context(), &StorageError::Codec, "{label}");

        assert_eq!(
            store.get::<u8>(["probe", "mode"]).unwrap(),
            None,
            "{label}: the refused write reached the document"
        );
    }
}

#[cfg(feature = "ron")]
#[test]
fn the_refusal_says_which_shape_and_why() {
    let path = TempPath::new("enum_refused_report");
    let store = StoreBuilder::new(path.path())
        .backend(Backend::Ron)
        .build()
        .unwrap();

    let refused = store
        .set(["probe", "mode"], &Mode::On(3))
        .expect_err("ron took an enum");

    assert_eq!(refused.current_context(), &StorageError::Codec);

    let rendered = format!("{refused:?}");
    assert!(rendered.contains("an enum"), "{rendered}");
    assert!(rendered.contains("ron-rs/ron/issues/122"), "{rendered}");
}

#[cfg(all(feature = "redb", feature = "ron"))]
#[test]
fn a_promise_to_stay_readable_on_ron_refuses_an_enum_where_it_would_fit() {
    let path = TempPath::new("enum_promised");
    let store = StoreBuilder::new(path.path())
        .backend(Backend::Redb)
        .limits(|l| l.portable_across([Backend::Ron]))
        .build()
        .unwrap();

    let refused = store
        .set(["probe", "mode"], &Mode::Off)
        .expect_err("msgpack holds it, but the store promised ron too");

    assert!(format!("{refused:?}").contains("an enum"), "{refused:?}");
}
