use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod keyed_v1 {
    use super::*;

    #[amethystate(prefix = "keyed", version = 1)]
    pub struct Keyed {
        #[amestate(key = "panels.left.visible", default = true)]
        pub left_panel_visible: bool,
    }
}

#[amethystate(prefix = "keyed", version = 2)]
pub struct Keyed {
    #[amestate(default = true)]
    pub panel_visible: bool,
}

#[migrate]
#[rename(left_panel_visible => panel_visible)]
fn migrate_keyed_v1_to_v2(
    old: AmeData<keyed_v1::Keyed>,
) -> amethystate::MigrationResult<AmeData<Keyed>> {
    Ok(AmeData::<Keyed> {
        panel_visible: old.left_panel_visible,
    })
}

#[backends(all)]
#[ignore = "known: migration cleanup addresses the Rust name, not the stored one - see TODO.md"]
fn renaming_a_field_that_carries_a_key_moves_its_stored_value(backend: Backend) {
    let path = TempPath::new("keyed_rename");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let v1 = keyed_v1::Keyed::new_with(&store).unwrap();
        v1.left_panel_visible().set(false).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();

        assert_eq!(
            store
                .get::<bool>(["keyed", "panels", "left", "visible"])
                .unwrap(),
            Some(false),
            "the override decides where v1 stores it"
        );
    }

    let (store, _report) = StoreBuilder::new(path.path())
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();

    let v2 = Keyed::new_with(&store).unwrap();
    assert!(!v2.panel_visible().get(), "value carried over");

    assert_eq!(
        store
            .get::<bool>(["keyed", "panels", "left", "visible"])
            .unwrap(),
        None,
        "the old location is cleaned up"
    );
}

mod dropped_v1 {
    use super::*;

    #[amethystate(prefix = "dropped", version = 1)]
    pub struct Dropped {
        #[amestate(default = 1u16)]
        pub kept: u16,

        #[amestate(key = "legacy.token", default = "remove-me".to_string())]
        pub legacy_token: String,
    }
}

#[amethystate(prefix = "dropped", version = 2)]
pub struct Dropped {
    #[amestate(default = 1u16)]
    pub kept: u16,
}

#[migrate]
fn migrate_dropped_v1_to_v2(
    old: AmeData<dropped_v1::Dropped>,
) -> amethystate::MigrationResult<AmeData<Dropped>> {
    Ok(AmeData::<Dropped> { kept: old.kept })
}

#[backends(all)]
#[ignore = "known: migration cleanup addresses the Rust name, not the stored one - see TODO.md"]
fn dropping_a_field_that_carries_a_key_removes_its_stored_value(backend: Backend) {
    let path = TempPath::new("keyed_drop");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let v1 = dropped_v1::Dropped::new_with(&store).unwrap();
        v1.legacy_token().set("secret".to_string()).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let (store, _report) = StoreBuilder::new(path.path())
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();

    assert_eq!(
        store.get::<String>(["dropped", "legacy", "token"]).unwrap(),
        None,
        "the field is gone from the schema, so its value goes too"
    );
}

mod plain_v1 {
    use super::*;

    #[amethystate(prefix = "plain", version = 1)]
    pub struct Plain {
        #[amestate(default = 1u16)]
        pub kept: u16,

        #[amestate(default = "remove-me".to_string())]
        pub legacy_token: String,
    }
}

#[amethystate(prefix = "plain", version = 2)]
pub struct Plain {
    #[amestate(default = 1u16)]
    pub kept: u16,
}

#[migrate]
fn migrate_plain_v1_to_v2(
    old: AmeData<plain_v1::Plain>,
) -> amethystate::MigrationResult<AmeData<Plain>> {
    Ok(AmeData::<Plain> { kept: old.kept })
}

#[backends(all)]
fn dropping_a_plain_field_removes_its_stored_value(backend: Backend) {
    let path = TempPath::new("plain_drop");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let v1 = plain_v1::Plain::new_with(&store).unwrap();
        v1.legacy_token().set("secret".to_string()).unwrap();
        store.flush_prefix(StorePath::root()).unwrap();
    }

    let (store, _report) = StoreBuilder::new(path.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();

    assert_eq!(
        store.get::<String>(["plain", "legacy_token"]).unwrap(),
        None,
        "control: without a key override the cleanup lands"
    );
}
