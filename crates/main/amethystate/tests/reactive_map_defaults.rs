use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

mod v1 {
    use super::*;
    #[amethystate(prefix = "app", version = 1)]
    pub struct AppConfig {
        #[amestate(default = {
            "HTTP_PROXY": "http://127.0.0.1:8080".to_string(),
            "NO_PROXY": "localhost".to_string()
        })]
        pub env: ReactiveMap<String, String>,
    }
}

#[amethystate(prefix = "app", version = 2)]
pub struct AppConfig {
    #[amestate(default = {
        "HTTP_PROXY": "http://127.0.0.1:8080".to_string(),
        "NO_PROXY": "localhost".to_string(),
        "NEW_KEY": "new_default".to_string()
    })]
    pub env: ReactiveMap<String, String>,
}

#[backends(all)]
fn test_map_defaults_applied_only_on_first_init(backend: Backend) {
    let path = TempPath::new("first_init");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::AppConfig::new_with(&store).unwrap();

        let env = config.env();
        assert_eq!(
            env.get("HTTP_PROXY"),
            Some("http://127.0.0.1:8080".to_string())
        );
        assert_eq!(env.get("NO_PROXY"), Some("localhost".to_string()));
    }

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::AppConfig::new_with(&store).unwrap();

        let env = config.env();
        assert_eq!(
            env.get("HTTP_PROXY"),
            Some("http://127.0.0.1:8080".to_string())
        );
        assert_eq!(env.get("NO_PROXY"), Some("localhost".to_string()));
    }
}

#[backends(all)]
fn test_deleted_map_key_does_not_resurrect(backend: Backend) {
    let path = TempPath::new("no_resurrect");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::AppConfig::new_with(&store).unwrap();
        config.env().remove("NO_PROXY").unwrap();
    }

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::AppConfig::new_with(&store).unwrap();
        assert_eq!(config.env().get("NO_PROXY"), None);
    }
}

#[backends(all)]
fn test_new_defaults_applied_on_version_upgrade(backend: Backend) {
    let path = TempPath::new("version_upgrade");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::AppConfig::new_with(&store).unwrap();
        config.env().remove("NO_PROXY").unwrap();
    }

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = AppConfig::new_with(&store).unwrap();
        let env = config.env();

        assert_eq!(env.get("NO_PROXY"), None);

        assert_eq!(
            env.get("HTTP_PROXY"),
            Some("http://127.0.0.1:8080".to_string())
        );

        assert_eq!(env.get("NEW_KEY"), None);
    }
}

#[backends(all)]
fn test_user_set_value_not_overwritten_by_defaults(backend: Backend) {
    let path = TempPath::new("no_overwrite");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::AppConfig::new_with(&store).unwrap();
        config
            .env()
            .insert("HTTP_PROXY".to_string(), &"http://custom:9999".to_string())
            .unwrap();
    }

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = v1::AppConfig::new_with(&store).unwrap();
        assert_eq!(
            config.env().get("HTTP_PROXY"),
            Some("http://custom:9999".to_string())
        );
    }
}
