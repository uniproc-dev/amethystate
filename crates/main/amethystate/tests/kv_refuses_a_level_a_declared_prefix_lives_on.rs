use amethystate::amethystate;
use amethystate::store::KvWrite;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "ui.panels")]
pub struct Panels {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[backends(all)]
fn kv_refuses_a_level_a_declared_prefix_lives_on(backend: Backend) {
    let at = TempPath::new("kv_over_ui");
    let store = StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap();
    let _panels = Panels::new_with(&store).unwrap();

    let refused = store.kv().set("ui", &1u32).unwrap_err();

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
        ("ui".to_string(), "ui.panels.width".to_string()),
        "{backend:?}"
    );
    assert!(by.ends_with("Panels"), "{backend:?}: {by}");
}
