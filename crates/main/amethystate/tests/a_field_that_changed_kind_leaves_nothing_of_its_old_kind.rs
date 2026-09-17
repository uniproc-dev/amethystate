use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, ReactiveMap, migrate};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "tally", version = 1)]
    pub struct Tally {
        pub count: u32,
    }

    #[amethystate(prefix = "sizes", version = 1)]
    pub struct Sizes {
        pub widths: ReactiveMap<String, u32>,
    }

    #[amethystate(prefix = "panel", version = 1)]
    pub struct Panel {
        #[amestate(nested)]
        pub layout: Layout,
    }

    #[amethystate]
    pub struct Layout {
        pub gap: u32,
        pub legacy: u32,
    }
}

#[amethystate(prefix = "tally", version = 2)]
pub struct Tally {
    pub count: ReactiveMap<String, u32>,
}

#[amethystate(prefix = "sizes", version = 2)]
pub struct Sizes {
    pub widths: u32,
}

#[amethystate(prefix = "panel", version = 2)]
pub struct Panel {
    #[amestate(nested)]
    pub layout: Layout,
}

#[amethystate]
pub struct Layout {
    pub gap: ReactiveMap<String, u32>,
}

#[migrate]
fn migrate_tally_v1_to_v2(old: AmeData<v1::Tally>) -> amethystate::MigrationResult<AmeData<Tally>> {
    Ok(AmeData::<Tally> {
        count: [("total".to_string(), old.count)].into_iter().collect(),
    })
}

#[migrate]
fn migrate_sizes_v1_to_v2(old: AmeData<v1::Sizes>) -> amethystate::MigrationResult<AmeData<Sizes>> {
    Ok(AmeData::<Sizes> {
        widths: old.widths.values().sum(),
    })
}

#[migrate]
fn migrate_panel_v1_to_v2(old: AmeData<v1::Panel>) -> amethystate::MigrationResult<AmeData<Panel>> {
    Ok(AmeData::<Panel> {
        layout: AmeData::<Layout> {
            gap: [("total".to_string(), old.layout.gap)]
                .into_iter()
                .collect(),
        },
    })
}

fn migrated(at: &TempPath, backend: Backend) -> amethystate::Store {
    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .migrate()
        .unwrap();

    assert!(!report.has_failures(), "{backend:?}: {report:?}");
    store
}

#[backends(all)]
fn a_value_that_became_a_map_leaves_no_value_behind(backend: Backend) {
    let at = TempPath::new("value_became_map");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        let tally = v1::Tally::new_with(&store).unwrap();
        tally.count().set(5).unwrap();
        store.save_now().unwrap();
    }

    let store = migrated(&at, backend);

    assert!(
        !matches!(store.get::<u32>(["tally", "count"]), Ok(Some(_))),
        "{backend:?}: the old value outlived the field becoming a map"
    );

    let tally = Tally::new_with(&store).unwrap();
    assert_eq!(tally.count().get("total"), Some(5));
    assert_eq!(tally.count().len(), 1);
}

#[backends(all)]
fn a_map_that_became_a_value_leaves_no_entry_behind(backend: Backend) {
    let at = TempPath::new("map_became_value");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        let sizes = v1::Sizes::new_with(&store).unwrap();
        sizes.widths().insert("cpu".into(), &110).unwrap();
        sizes.widths().insert("gpu".into(), &120).unwrap();
        store.save_now().unwrap();
    }

    let store = migrated(&at, backend);

    assert_eq!(
        store
            .scan_keys(StorePath::from_segments(["sizes", "widths"]))
            .unwrap(),
        vec![StorePath::from_segments(["sizes", "widths"])],
        "{backend:?}: an entry outlived the map becoming a value"
    );

    let sizes = Sizes::new_with(&store).unwrap();
    assert_eq!(sizes.widths().get(), 230);
}

#[backends(all)]
fn a_nested_struct_leaves_nothing_of_a_field_it_changed_or_gave_up(backend: Backend) {
    let at = TempPath::new("nested_changed_kind");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        let panel = v1::Panel::new_with(&store).unwrap();
        panel.layout().gap().set(4).unwrap();
        panel.layout().legacy().set(9).unwrap();
        store.save_now().unwrap();
    }

    let store = migrated(&at, backend);

    assert!(
        !matches!(store.get::<u32>(["panel", "layout", "gap"]), Ok(Some(_))),
        "{backend:?}: the old value outlived a nested field becoming a map"
    );
    assert_eq!(
        store.get::<u32>(["panel", "layout", "legacy"]).unwrap(),
        None,
        "{backend:?}: a field the nested struct gave up outlived the migration"
    );

    let panel = Panel::new_with(&store).unwrap();
    assert_eq!(panel.layout().gap().get("total"), Some(4));
}
