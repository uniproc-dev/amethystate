use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(as_root)]
pub struct AppConfig {
    #[amestate(default = "legacy".to_string())]
    pub name: String,

    #[amestate(default = false)]
    pub comfy: bool,
}

#[backends(all)]
fn a_root_struct_lays_its_fields_at_the_top(backend: Backend) {
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
