use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, ReactiveMap, migrate};
use amethystate_core::test_utils::unique_path;
use amethystate_macros::{AmeType, amethystate};
use amethystate_test_macros::backends;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, AmeType)]
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
    ctx: &mut amethystate::migration::MigrationContext,
) -> amethystate::MigrationResult<AmeData<ProxyConfig>> {
    for key in old.routes.keys() {
        ctx.delete(&format!("routes.{}", key))?;
    }

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

#[backends(all)]
fn test_embedded_map_migration(backend: Backend) {
    let path = unique_path("amethystate_embedded_map");

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
