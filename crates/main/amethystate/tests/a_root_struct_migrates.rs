use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(as_root, version = 1)]
    pub struct AppConfig {
        #[amestate(default = "legacy".to_string())]
        pub name: String,
    }
}

#[amethystate(as_root, version = 2)]
pub struct AppConfig {
    #[amestate(default = "legacy".to_string())]
    pub title: String,
}

#[migrate]
#[rename(name => title)]
fn migrate_app_config_v1_to_v2(
    old: AmeData<v1::AppConfig>,
) -> amethystate::MigrationResult<AmeData<AppConfig>> {
    Ok(AmeData::<AppConfig> { title: old.name })
}

#[backends(all)]
fn a_struct_at_the_root_is_migrated_like_any_other(backend: Backend) {
    let at = TempPath::new("root_struct_migrates");

    {
        let store = StoreBuilder::new(at.path())
            .backend(backend)
            .build()
            .unwrap();
        let config = v1::AppConfig::new_with(&store).unwrap();
        config.name().set("kept".to_string()).unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(at.path())
        .backend(backend)
        .migrations(|m| {
            m.collect_codegen();
        })
        .build_with_migration()
        .unwrap();

    assert!(!report.has_failures(), "{report:?}");

    let config = AppConfig::new_with(&store).unwrap();
    assert_eq!(config.title().get(), "kept");
}
