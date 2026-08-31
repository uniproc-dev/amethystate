#[cfg(any(feature = "json", feature = "toml"))]
use amethystate::store::StorageError;
use amethystate::store::builder::StoreBuilder;
use amethystate_core::test_utils::TempPath;

mod common;

fn store_on(backend: amethystate::store::builder::Backend, at: &TempPath) -> amethystate::Store {
    StoreBuilder::new(at.path()).backend(backend).build().unwrap()
}

#[test]
fn an_engine_that_cannot_tell_it_from_none_refuses_it() {
    for backend in common::enabled_backends() {
        if backend.keeps_a_nested_option() {
            continue;
        }

        let path = TempPath::new("nested_option");
        let store = store_on(backend, &path);

        let refused = store
            .set(["probe", "v"], &Some(None::<u32>))
            .expect_err(&format!(
                "{backend:?} took a Some(None) it reads back as None"
            ));

        assert_eq!(
            store.get::<Option<Option<u32>>>(["probe", "v"]).unwrap(),
            None,
            "{backend:?}: the refused write reached the store"
        );
        let _ = refused;
    }
}

#[test]
fn the_engine_that_keeps_it_takes_it() {
    for backend in common::enabled_backends() {
        if !backend.keeps_a_nested_option() {
            continue;
        }

        let path = TempPath::new("nested_option_ok");
        let store = store_on(backend, &path);

        store.set(["probe", "v"], &Some(None::<u32>)).unwrap();
        assert_eq!(
            store.get::<Option<Option<u32>>>(["probe", "v"]).unwrap(),
            Some(Some(None)),
            "{backend:?}"
        );
    }
}

#[test]
fn a_plain_none_is_not_refused() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("plain_none");
        let store = store_on(backend, &path);

        store
            .set(["probe", "v"], &None::<u32>)
            .unwrap_or_else(|e| panic!("{backend:?}: {e:?}"));
    }
}

#[test]
fn a_some_holding_something_that_holds_a_none_is_not_refused() {
    for backend in common::enabled_backends() {
        if common::engine_name(backend) == "toml" {
            continue;
        }

        let path = TempPath::new("some_of_a_list");
        let store = store_on(backend, &path);

        let value = Some(vec![None::<u32>, Some(1)]);
        store
            .set(["probe", "list"], &value)
            .unwrap_or_else(|e| panic!("{backend:?} refused a None two levels down: {e:?}"));

        assert_eq!(
            store
                .get::<Option<Vec<Option<u32>>>>(["probe", "list"])
                .unwrap(),
            Some(value),
            "{backend:?}"
        );
    }
}

#[test]
fn a_struct_field_that_is_none_is_not_refused() {
    for backend in common::enabled_backends() {
        let path = TempPath::new("none_field");
        let store = store_on(backend, &path);

        let value = Some(1u32);
        store
            .set(["probe", "opt"], &value)
            .unwrap_or_else(|e| panic!("{backend:?}: {e:?}"));

        assert_eq!(
            store.get::<Option<u32>>(["probe", "opt"]).unwrap(),
            Some(Some(1)),
            "{backend:?}"
        );
    }
}

#[cfg(feature = "json")]
#[test]
fn the_refusal_says_what_it_is_and_why() {
    use amethystate::store::builder::Backend;

    let path = TempPath::new("nested_option_report");
    let store = store_on(Backend::Json, &path);

    let refused = store
        .set(["probe", "v"], &Some(None::<u32>))
        .expect_err("json took it");

    assert_eq!(refused.current_context(), &StorageError::Codec);

    let rendered = format!("{refused:?}");
    assert!(rendered.contains("holding nothing"), "{rendered}");
    assert!(rendered.contains("ron"), "{rendered}");
}

#[cfg(feature = "toml")]
#[test]
fn toml_refuses_it_in_its_own_codec_before_the_screening_looks() {
    use amethystate::store::builder::Backend;

    let path = TempPath::new("nested_option_toml");
    let store = store_on(Backend::Toml, &path);

    let refused = store
        .set(["probe", "v"], &Some(None::<u32>))
        .expect_err("toml took it");

    assert_eq!(refused.current_context(), &StorageError::Write);
}

#[cfg(all(feature = "ron", feature = "json"))]
#[test]
fn a_promise_to_stay_readable_on_json_refuses_it_on_ron() {
    use amethystate::store::builder::Backend;

    let path = TempPath::new("nested_option_promised");
    let store = StoreBuilder::new(path.path())
        .backend(Backend::Ron)
        .limits(|l| l.portable_across([Backend::Json]))
        .build()
        .unwrap();

    let refused = store
        .set(["probe", "v"], &Some(None::<u32>))
        .expect_err("ron keeps it, but the store promised json too");

    assert!(
        format!("{refused:?}").contains("holding nothing"),
        "{refused:?}"
    );
}
