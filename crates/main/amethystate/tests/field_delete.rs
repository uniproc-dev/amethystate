use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;
use std::sync::{Arc, Mutex};

#[amethystate(prefix = "del", on_delete = UseDefault)]
pub struct Cfg {
    #[amestate(default = 7u64)]
    pub counter: u64,
}

#[backends(all)]
fn a_deleted_key_falls_back_to_the_default(backend: Backend) {
    let path = unique_path("field_delete");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    cfg.counter().set(42).unwrap();
    assert_eq!(cfg.counter().get(), 42);

    store.delete(["del", "counter"]).unwrap();

    assert_eq!(store.get::<u64>(["del", "counter"]).unwrap(), None);
    assert_eq!(cfg.counter().get(), 7, "field must not outlive the key");
}

#[backends(all)]
fn a_delete_notifies_subscribers(backend: Backend) {
    let path = unique_path("field_delete_notify");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let cap = seen.clone();
    let _sub = cfg
        .counter()
        .subscribe(move |v: &u64| cap.lock().unwrap().push(*v));

    cfg.counter().set(42).unwrap();
    store.delete(["del", "counter"]).unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![42, 7]);
}

#[backends(all)]
fn writing_again_after_a_delete_works(backend: Backend) {
    let path = unique_path("field_delete_rewrite");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();

    store.delete(["del", "counter"]).unwrap();
    cfg.counter().set(5).unwrap();

    assert_eq!(cfg.counter().get(), 5);
    assert_eq!(store.get::<u64>(["del", "counter"]).unwrap(), Some(5));
}
