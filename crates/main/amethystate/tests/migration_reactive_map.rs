use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, ReactiveMap, migrate};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProxyEndpoint {
    pub url: String,
    pub timeout_ms: u32,
}

mod v1 {
    use super::*;
    #[amethystate(prefix = "network", version = 1)]
    pub struct ProxyConfig {
        #[amestate(default = "default".into())]
        pub name: String,
        pub routes: ReactiveMap<String, String>,
    }
}

#[amethystate(prefix = "network", version = 2)]
pub struct ProxyConfig {
    #[amestate(default = "default".into())]
    pub name: String,
    pub endpoints: ReactiveMap<String, ProxyEndpoint>,
}

#[migrate]
fn migrate_proxy_config_v1_to_v2(
    old: AmeData<v1::ProxyConfig>,
    _ctx: &mut amethystate::migration::MigrationContext,
) -> amethystate::MigrationResult<AmeData<ProxyConfig>> {
    let endpoints = old
        .routes
        .into_iter()
        .filter(|(k, _)| k != "obsolete")
        .map(|(k, v)| {
            (
                k,
                ProxyEndpoint {
                    url: v,
                    timeout_ms: 5000,
                },
            )
        })
        .collect();

    Ok(AmeData::<ProxyConfig> {
        name: old.name,
        endpoints,
    })
}

mod emptied_v1 {
    use super::*;

    #[amethystate(prefix = "emptied", version = 1)]
    pub struct Held {
        #[amestate(default = 0u32)]
        pub counted: u32,

        pub cache: ReactiveMap<String, u32>,
    }
}

#[amethystate(prefix = "emptied", version = 2)]
pub struct Held {
    #[amestate(default = 0u32)]
    pub counted: u32,

    pub cache: ReactiveMap<String, u32>,
}

#[migrate]
fn migrate_emptied_v1_to_v2(
    _old: AmeData<emptied_v1::Held>,
    ctx: &mut amethystate::migration::MigrationContext,
) -> amethystate::MigrationResult<AmeData<Held>> {
    let seen = ctx.scan_map::<String, u32>("cache")?;

    Ok(AmeData::<Held> {
        counted: seen.len() as u32,
        cache: seen.into_iter().collect(),
    })
}

#[backends(all)]
fn a_step_scans_a_map_somebody_emptied(backend: Backend) {
    let path = TempPath::new("emptied_map_scan");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let held = emptied_v1::Held::new_with(&store).unwrap();
        held.cache().insert("a".into(), &1u32).unwrap();
        held.cache().remove("a").unwrap();
        store.save_now().unwrap();
    }

    let (store, report) = StoreBuilder::new(&path)
        .backend(backend)
        .build_with_migration()
        .unwrap();

    assert!(
        !report.has_failures(),
        "the step scanned a map with nothing in it and the scan refused the map's own \
         level: {:?}",
        report.components
    );

    let held = Held::new_with(&store).unwrap();
    assert_eq!(
        held.counted().get(),
        0,
        "the emptied map read back as empty"
    );
}

#[backends(all)]
fn test_embedded_map_migration(backend: Backend) {
    let path = TempPath::new("amethystate_embedded_map");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::ProxyConfig::new_with(&store).unwrap();
        config.name().set("legacy-proxy".into()).unwrap();

        config
            .routes()
            .insert("api".into(), &"http://api.v1".into())
            .unwrap();
        config
            .routes()
            .insert("obsolete".into(), &"http://drop.me".into())
            .unwrap();
        store.save_now().unwrap();
    }

    let (store, _) = StoreBuilder::new(&path)
        .backend(backend)
        .build_with_migration()
        .unwrap();

    let config = ProxyConfig::new_with(&store).unwrap();

    assert_eq!(config.name().get(), "legacy-proxy");

    let entries: Vec<_> = config.endpoints().entries().collect();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "api");
    assert_eq!(entries[0].1.url, "http://api.v1");

    let old_keys = store.scan_prefix(["network", "routes"]).unwrap();
    assert!(old_keys.is_empty());
}
