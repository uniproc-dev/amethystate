use amethystate::amethystate;
#[cfg(feature = "redb")]
use amethystate::store::InspectorBackend;
#[cfg(feature = "redb")]
use amethystate::store::builder::{Backend, StoreBuilder};
#[cfg(feature = "redb")]
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "ui", version = 1)]
pub struct UiColors {
    #[amestate(default = 0u32)]
    pub accent: u32,
}

#[amethystate(prefix = "ui", version = 1)]
pub struct UiLayout {
    #[amestate(default = 1u32)]
    pub density: u32,
}

#[cfg(feature = "redb")]
#[backends(Redb)]
fn two_schemas_at_one_prefix_each_keep_their_own_snapshot(backend: Backend) {
    let path = TempPath::new("snapshot_per_prefix");
    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let _a = UiColors::new_with(&store).unwrap();
        let _b = UiLayout::new_with(&store).unwrap();
        store.save_now().unwrap();
    }

    let inspector = amethystate::stores::RedbStore::open(
        amethystate::StoreConfig::new(path.path()),
        Default::default(),
    )
    .unwrap()
    .0;
    let snapshots = inspector.get_schema_snapshots().unwrap();
    let named: Vec<_> = snapshots
        .iter()
        .filter(|(key, _)| key == "ui")
        .filter_map(|(_, s)| s.struct_name.as_deref())
        .collect();

    assert_eq!(
        named.len(),
        2,
        "one of the two schemas at `ui` has no snapshot: {named:?}"
    );
}
