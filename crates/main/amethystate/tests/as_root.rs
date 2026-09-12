use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use serde::{Deserialize, Serialize};

#[amethystate(as_root)]
pub struct AppConfig {
    #[amestate(default = "legacy".to_string())]
    pub name: String,

    #[amestate(default = false)]
    pub comfy: bool,
}

#[backends(all)]
fn test_as_root_global_namespace(backend: Backend) {
    let path = TempPath::new("as_root_test");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let config = AppConfig::new_with(&store).unwrap();

    assert_eq!(
        store.get::<String>(["name"]).unwrap(),
        Some("legacy".to_string())
    );
    assert_eq!(store.get::<bool>(["comfy"]).unwrap(), Some(false));

    config.name().set("updated_name".to_string()).unwrap();
    config.comfy().set(true).unwrap();

    assert_eq!(
        store.get::<String>(["name"]).unwrap(),
        Some("updated_name".to_string())
    );
    assert_eq!(store.get::<bool>(["comfy"]).unwrap(), Some(true));
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Net {
    pub host: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Written {
    pub name: String,
    pub comfy: bool,
    pub net: Net,
}

#[amethystate(as_root)]
pub struct Spelled {
    #[amestate(default = "legacy".to_string())]
    pub name: String,

    #[amestate(default = false)]
    pub comfy: bool,

    #[amestate(default = Net { host: "h".to_string(), port: 1 })]
    pub net: Net,
}

#[cfg(feature = "json")]
#[test]
fn a_root_struct_is_the_file_serde_would_have_written() {
    let path = TempPath::new("as_root_serde");
    let store = StoreBuilder::new(&path)
        .backend(Backend::Json)
        .build()
        .unwrap();

    let config = Spelled::new_with(&store).unwrap();
    config.name().set("given".to_string()).unwrap();
    store.save_now().unwrap();

    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path.path()).unwrap()).unwrap();

    assert_eq!(
        on_disk,
        serde_json::to_value(Written {
            name: "given".to_string(),
            comfy: false,
            net: Net {
                host: "h".to_string(),
                port: 1
            },
        })
        .unwrap()
    );
}

#[cfg(feature = "json")]
#[test]
fn a_key_under_a_root_field_is_a_key_beside_it() {
    let path = TempPath::new("as_root_beside");
    let store = StoreBuilder::new(&path)
        .backend(Backend::Json)
        .build()
        .unwrap();

    let _config = Spelled::new_with(&store).unwrap();
    store.set(["name", "inner"], &1u32).unwrap();
    store.save_now().unwrap();

    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path.path()).unwrap()).unwrap();

    assert_eq!(on_disk["name"], serde_json::json!("legacy"));
    assert_eq!(on_disk["name.inner"], serde_json::json!(1));
}

#[backends(all)]
fn a_key_under_a_root_field_leaves_that_field_alone(backend: Backend) {
    let path = TempPath::new("as_root_beside_all");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let _config = Spelled::new_with(&store).unwrap();
    store.set(["name", "inner"], &1u32).unwrap();

    assert_eq!(
        store.get::<String>(["name"]).unwrap(),
        Some("legacy".to_string()),
        "a key under a declared field must not reach inside its value"
    );
    assert_eq!(store.get::<u32>(["name", "inner"]).unwrap(), Some(1));
    assert_eq!(
        store.get::<Net>(["net"]).unwrap(),
        Some(Net {
            host: "h".to_string(),
            port: 1
        })
    );
}
