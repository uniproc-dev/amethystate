use amethystate::amethystate;
use amethystate::store::KvWrite;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "look")]
pub struct Look {
    #[amestate(path = "font.size", default = 14u32)]
    pub font_size: u32,
}

#[backends(all)]
fn kv_refuses_a_level_a_declared_field_lives_on(backend: Backend) {
    let at = TempPath::new("kv_over_look_font");
    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap();
    let _look = Look::new_with(&store).unwrap();

    let refused = store.kv().namespace("look").set("font", &1u32).unwrap_err();

    let KvWrite::Declared {
        at,
        declared_at,
        by,
    } = refused
    else {
        panic!("{backend:?}: {refused}");
    };

    assert_eq!(
        (at.to_string(), declared_at.to_string()),
        ("look.font".to_string(), "look.font.size".to_string()),
        "{backend:?}"
    );
    assert!(by.ends_with("Look"), "{backend:?}: {by}");
}
