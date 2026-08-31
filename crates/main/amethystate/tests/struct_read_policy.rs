use amethystate::amethystate;
use amethystate::errors::StorageError;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[amethystate(prefix = "strict")]
pub struct Strict {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,
}

#[amethystate(prefix = "lenient", on_unreadable = UseDefault)]
pub struct Lenient {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "127.0.0.1".to_string())]
    pub host: String,
}

//@show a struct that opens over a value it cannot read
#[amethystate(prefix = "mixed", on_unreadable = UseDefault)]
pub struct Mixed {
    #[amestate(default = 8080u16)]
    pub port: u16,

    #[amestate(default = "".to_string(), on_unreadable = Refuse)]
    pub licence: String,
}
//@show-end

#[backends(all)]
fn an_undecodable_change_leaves_the_last_value_alone(backend: Backend) {
    let path = TempPath::new("read_policy_live");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let state = Strict::new_with(&store).unwrap();

    state.port().set(9090).unwrap();

    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let _sub = state.port().subscribe(move |_| {
        count.fetch_add(1, Ordering::Release);
    });

    store
        .set(["strict", "port"], &"not a number".to_string())
        .unwrap();

    assert_eq!(state.port().get(), 9090);
    assert!(state.port().try_get().is_err());
    assert_eq!(woken.load(Ordering::Acquire), 0);
}

#[backends(all)]
fn a_change_that_decodes_is_delivered_again(backend: Backend) {
    let path = TempPath::new("read_policy_live_recovers");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let state = Strict::new_with(&store).unwrap();

    store
        .set(["strict", "port"], &"not a number".to_string())
        .unwrap();
    assert!(state.port().try_get().is_err());

    store.set(["strict", "port"], &1234u16).unwrap();

    assert_eq!(state.port().try_get().unwrap(), 1234);
}

#[backends(all)]
fn a_field_may_demand_more_than_the_struct_promised(backend: Backend) {
    let path = TempPath::new("read_policy_mixed_licence");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["mixed", "licence"], &7u32).unwrap();

    assert!(Mixed::new_with(&store).is_err());
}

#[backends(all)]
fn the_struct_rule_still_covers_the_fields_that_did_not_ask(backend: Backend) {
    let path = TempPath::new("read_policy_mixed_port");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store
        .set(["mixed", "port"], &"not a number".to_string())
        .unwrap();

    let state = Mixed::new_with(&store).unwrap();

    assert_eq!(state.port().get(), 8080);
    assert!(state.port().try_get().is_err());
}

#[backends(all)]
fn a_struct_refuses_to_open_over_a_value_it_cannot_read(backend: Backend) {
    let path = TempPath::new("read_policy_strict");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store
        .set(["strict", "port"], &"not a number".to_string())
        .unwrap();

    let refused = Strict::new_with(&store).unwrap_err();

    assert_eq!(refused.current_context(), &StorageError::Read);
}

#[backends(all)]
fn use_default_opens_and_the_field_says_the_store_disagrees(backend: Backend) {
    let path = TempPath::new("read_policy_lenient");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store
        .set(["lenient", "port"], &"not a number".to_string())
        .unwrap();

    let state = Lenient::new_with(&store).unwrap();

    assert_eq!(state.port().get(), 8080);
    assert!(state.port().try_get().is_err());

    assert_eq!(state.host().get(), "127.0.0.1");
    assert!(state.host().try_get().is_ok());
}

#[backends(all)]
fn use_default_leaves_the_stored_value_where_it_is(backend: Backend) {
    let path = TempPath::new("read_policy_untouched");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store
        .set(["lenient", "port"], &"not a number".to_string())
        .unwrap();
    let _state = Lenient::new_with(&store).unwrap();

    assert_eq!(
        store.get::<String>(["lenient", "port"]).unwrap(),
        Some("not a number".to_string())
    );
}

#[backends(all)]
fn a_write_that_decodes_clears_the_complaint(backend: Backend) {
    let path = TempPath::new("read_policy_recovers");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store
        .set(["lenient", "port"], &"not a number".to_string())
        .unwrap();
    let state = Lenient::new_with(&store).unwrap();

    assert!(state.port().try_get().is_err());

    state.port().set(9090).unwrap();

    assert_eq!(state.port().try_get().unwrap(), 9090);
}

#[backends(all)]
fn a_readable_store_is_untouched_by_the_policy(backend: Backend) {
    let path = TempPath::new("read_policy_ordinary");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let state = Lenient::new_with(&store).unwrap();

    assert_eq!(state.port().try_get().unwrap(), 8080);
    assert_eq!(state.host().try_get().unwrap(), "127.0.0.1");
}
