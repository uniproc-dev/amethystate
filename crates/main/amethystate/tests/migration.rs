use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate};
use amethystate_core::test_utils::unique_path;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate(prefix = "app", version = 1)]
    pub struct Config {
        #[amestate(default = "localhostv1".to_string())]
        pub host: String,
    }
}

#[amethystate(prefix = "app", version = 2)]
pub struct Config {
    #[amestate(default = "localhostv2".to_string())]
    pub address: String,

    #[amestate(default = 8080)]
    pub port: u16,
}

#[migrate]
#[rename(host => address)]
fn migrate_config_v1_to_v2(
    old: AmeData<v1::Config>,
) -> amethystate::MigrationResult<AmeData<Config>> {
    Ok(AmeData::<Config> {
        address: old.host,
        port: 9090,
    })
}

#[backends(all)]
fn test_decentralized_codegen_migration(backend: Backend) {
    let path = unique_path("amethystate_integration_test");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::Config::new_with(&store).unwrap();
        config.host().set("10.0.0.1".to_string()).unwrap();
    }

    let (store, reports) = StoreBuilder::new(&path)
        .backend(backend)
        .build_with_migration()
        .unwrap();

    assert!(!reports.has_failures());

    let config = Config::new_with(&store).expect("Failed to create Config");

    assert_eq!(config.address().get(), "10.0.0.1");

    assert_eq!(config.port().get(), 9090);

    let old_val: Option<String> = store.get(["app", "host"]).unwrap();
    assert!(
        old_val.is_none(),
        "Old key 'app.host' should be deleted after migration"
    );

    let new_val: Option<String> = store.get(["app", "address"]).unwrap();
    assert_eq!(new_val, Some("10.0.0.1".to_string()));
}
