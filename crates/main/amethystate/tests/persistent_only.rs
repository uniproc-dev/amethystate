use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;

#[amethystate(prefix = "network", mode = "both")]
pub struct NetworkState {
    #[amestate(default = "localhost".to_string())]
    pub host: String,

    #[amestate(default = 8080)]
    pub port: u16,
}

#[backends(all)]
fn persistent_only_load_save_and_mutate(backend: Backend) {
    let path = unique_path("persistent-only");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let state = NetworkState::new_with(&store).unwrap();
    state.host().set("10.0.0.1".to_string()).unwrap();
    state.port().set(3030).unwrap();

    let mut data = NetworkState::load_with(&store).unwrap();
    assert_eq!(data.host, "10.0.0.1");
    assert_eq!(data.port, 3030);

    data.port = 9090;
    data.save().unwrap();
    assert_eq!(store.get::<u16>(["network", "port"]).unwrap(), Some(9090));

    data.mutate(|d| {
        d.host = "127.0.0.1".to_string();
        d.port = 4040;
    })
    .unwrap();

    assert_eq!(
        store.get::<String>(["network", "host"]).unwrap(),
        Some("127.0.0.1".to_string())
    );
    assert_eq!(store.get::<u16>(["network", "port"]).unwrap(), Some(4040));
}
