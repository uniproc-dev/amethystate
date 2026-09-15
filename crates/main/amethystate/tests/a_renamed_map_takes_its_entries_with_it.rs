use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, ReactiveMap, migrate};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "cols", version = 1)]
    pub struct Panel {
        pub widths: ReactiveMap<String, u32>,
    }
}

#[amethystate(prefix = "cols", version = 2)]
pub struct Panel {
    pub sizes: ReactiveMap<String, u32>,
}

#[migrate]
#[rename(widths => sizes)]
fn migrate_panel_v1_to_v2(old: AmeData<v1::Panel>) -> amethystate::MigrationResult<AmeData<Panel>> {
    Ok(AmeData::<Panel> { sizes: old.widths })
}

#[backends(all)]
fn a_renamed_map_keeps_every_entry_and_leaves_nothing_at_the_old_place(backend: Backend) {
    let at = TempPath::new("renamed_map");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        let panel = v1::Panel::new_with(&store).unwrap();
        panel.widths().insert("cpu".into(), &110).unwrap();
        panel.widths().insert("gpu".into(), &120).unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();

    assert!(!report.has_failures(), "{backend:?}: {report:?}");

    assert_eq!(
        store.get::<u32>(["cols", "sizes", "cpu"]).unwrap(),
        Some(110),
        "{backend:?}: an entry did not come across with the map"
    );
    assert_eq!(
        store.get::<u32>(["cols", "sizes", "gpu"]).unwrap(),
        Some(120),
        "{backend:?}: an entry did not come across with the map"
    );

    assert_eq!(
        store
            .scan_keys(StorePath::from_segments(["cols", "widths"]))
            .unwrap(),
        Vec::<StorePath>::new(),
        "{backend:?}: the map's old subtree outlived the rename"
    );

    let panel = Panel::new_with(&store).unwrap();
    assert_eq!(panel.sizes().len(), 2);
}
