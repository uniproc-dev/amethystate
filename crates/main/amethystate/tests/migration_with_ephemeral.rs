use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, migrate, migrate_field};
use amethystate_core::test_utils::unique_path;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;

    #[amethystate]
    pub struct NetworkSettings {
        #[amestate(default = 8080)]
        pub port: u16,
    }

    #[amethystate(prefix = "system", version = 1)]
    pub struct SystemConfig {
        #[amestate(nested)]
        pub net: NetworkSettings,
    }

    #[amethystate(prefix = "ui")]
    pub struct Dashboard {
        #[amestate(default = false, volatile)]
        pub is_loading: bool,
    }
}

#[amethystate]
pub struct NetworkSettings {
    #[amestate(default = 8080)]
    pub listen_port: u16,
}

#[amethystate(prefix = "system", version = 2)]
pub struct SystemConfig {
    #[amestate(nested)]
    pub net: NetworkSettings,
}

#[amethystate(prefix = "ui")]
pub struct Dashboard {
    #[amestate(default = false, volatile)]
    pub is_loading: bool,
}

#[migrate]
#[rename(port => listen_port)]
fn migrate_network_settings_v1_to_v2(
    old: AmeData<v1::NetworkSettings>,
) -> amethystate::MigrationResult<AmeData<NetworkSettings>> {
    Ok(AmeData::<NetworkSettings> {
        listen_port: old.port,
    })
}

#[migrate]
fn migrate_system_config_v1_to_v2(
    old: AmeData<v1::SystemConfig>,
    ctx: &mut amethystate::migration::MigrationContext,
) -> amethystate::MigrationResult<AmeData<SystemConfig>> {
    Ok(AmeData::<SystemConfig> {
        net: migrate_field!(ctx, old.net),
    })
}

#[backends(all)]
fn test_nested_and_ephemeral_integration(backend: Backend) {
    let path = unique_path("amethystate_ephemeral_test");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

        let sys = v1::SystemConfig::new_with(&store).unwrap();
        let ui = v1::Dashboard::new_with(&store).unwrap();

        sys.net().port().set(9999).unwrap();
        ui.is_loading().set(true).unwrap();

        assert_eq!(sys.net().port().get(), 9999);
        assert!(ui.is_loading().get());

        store.save_now().unwrap();
    }

    {
        let (store, _) = StoreBuilder::new(&path)
            .backend(backend)
            .build_with_migration()
            .unwrap();

        let sys = SystemConfig::new_with(&store).expect("Failed to load v2 system");
        let ui = Dashboard::new_with(&store).expect("Failed to load dashboard");

        assert_eq!(sys.net().listen_port().get(), 9999);

        assert!(
            !ui.is_loading().get(),
            "a volatile field is never stored, so it comes back at its default"
        );

        let old_raw: Option<u16> = store.get(["system", "net", "port"]).unwrap();
        assert!(old_raw.is_none(), "Old nested key should be gone");

        let new_raw: Option<u16> = store.get(["system", "net", "listen_port"]).unwrap();
        assert_eq!(new_raw, Some(9999));
    }
}
