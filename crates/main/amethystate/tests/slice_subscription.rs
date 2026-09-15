use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[amethystate]
pub struct ServerConfig {
    #[amestate(default = "localhost".to_string())]
    pub host: String,

    #[amestate(default = 8080)]
    pub port: u16,
}

#[amethystate(prefix = "app")]
pub struct AppState {
    #[amestate(default = "admin".to_string())]
    pub username: String,

    #[amestate(nested)]
    pub server: ServerConfig,

    #[amestate(default = {})]
    pub tags: amethystate::ReactiveMap<String, String>,
}

#[backends(all)]
fn test_slice_subscribe_all(backend: Backend) {
    let path = TempPath::new("slice_sub_all");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let state = AppState::new_with(&store).unwrap();

    let change_count = Arc::new(AtomicUsize::new(0));
    let cc_clone = change_count.clone();

    let mut scope = state.subscribe_all(move || {
        cc_clone.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(change_count.load(Ordering::SeqCst), 0);

    state.username().set("superuser".to_string()).unwrap();
    assert_eq!(change_count.load(Ordering::SeqCst), 1);

    state.server().host().set("127.0.0.1".to_string()).unwrap();
    assert_eq!(change_count.load(Ordering::SeqCst), 2);

    state.tags().insert("env".into(), &"prod".into()).unwrap();
    assert_eq!(change_count.load(Ordering::SeqCst), 3);

    scope.clear();

    state.username().set("guest".to_string()).unwrap();
    assert_eq!(
        change_count.load(Ordering::SeqCst),
        3,
        "Changes must not be tracked after scope clearance"
    );
}

#[backends(all)]
fn test_slice_subscribe_all_external(backend: Backend) {
    let path = TempPath::new("slice_sub_all_ext");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let state = AppState::new_with(&store).unwrap();
    let fork = state.fork();

    let change_count = Arc::new(AtomicUsize::new(0));
    let cc_clone = change_count.clone();

    let _scope = state.subscribe_all_external(move || {
        cc_clone.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(change_count.load(Ordering::SeqCst), 0);

    state.username().set("superuser".to_string()).unwrap();
    assert_eq!(
        change_count.load(Ordering::SeqCst),
        0,
        "Own flat field changes must be ignored"
    );

    fork.username().set("fork_user".to_string()).unwrap();
    assert_eq!(
        change_count.load(Ordering::SeqCst),
        1,
        "Updates from fork must be processed"
    );

    state.server().port().set(9090).unwrap();
    assert_eq!(
        change_count.load(Ordering::SeqCst),
        1,
        "Own nested structure changes must be ignored"
    );

    fork.server().port().set(3000).unwrap();
    assert_eq!(
        change_count.load(Ordering::SeqCst),
        2,
        "Updates to nested structure from fork must be processed"
    );

    state.tags().insert("region".into(), &"eu".into()).unwrap();
    assert_eq!(
        change_count.load(Ordering::SeqCst),
        3,
        "New field creation (Insert) must not be ignored"
    );

    state.tags().update("region", &"us".into()).unwrap();
    assert_eq!(
        change_count.load(Ordering::SeqCst),
        3,
        "Own map updates (Update) must be ignored"
    );

    fork.tags().update("region", &"asia".into()).unwrap();
    assert_eq!(
        change_count.load(Ordering::SeqCst),
        4,
        "Map updates from fork (Update) must be processed"
    );
}
