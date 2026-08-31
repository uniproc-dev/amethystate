use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;
#[amethystate(as_root)]
pub struct AppConfig {
    #[amestate(default = "legacy".to_string())]
    pub name: String,

    #[amestate(default = false)]
    pub comfy: bool,
}

#[backends(all)]
fn test_as_root_global_namespace(backend: Backend) {
    let path = unique_path("as_root_test");
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
