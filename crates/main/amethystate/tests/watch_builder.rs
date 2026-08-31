use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{MapChange, ReactiveMap, amethystate};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;
use std::sync::{Arc, Mutex};

#[amethystate(prefix = "w")]
pub struct Cfg {
    #[amestate(default = 0u64)]
    pub counter: u64,

    #[amestate(default = {})]
    pub items: ReactiveMap<String, u64>,
}

fn cfg(backend: Backend) -> (impl amethystate::store::StoreBackend, Cfg) {
    let path = unique_path("watch_builder");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();
    (store, cfg)
}

#[backends(all)]
fn immediate_register_behaves_like_subscribe(backend: Backend) {
    let (_s, cfg) = cfg(backend);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let cap = seen.clone();

    let _sub = cfg.counter().subscription_with().register(move |v: &u64| {
        cap.lock().unwrap().push(*v);
    });

    cfg.counter().set(1).unwrap();
    cfg.counter().set(2).unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![1, 2]);
}

#[backends(all)]
fn external_and_with_source_compose(backend: Backend) {
    let (_s, cfg) = cfg(backend);
    let field = cfg.counter();
    let other = field.fork();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let cap = seen.clone();

    let _sub = field
        .subscription_with()
        .external()
        .register_with_source(move |v: &u64, src| {
            cap.lock().unwrap().push((*v, src.is_some()));
        });

    field.set(1).unwrap();
    other.set(2).unwrap();

    assert_eq!(
        *seen.lock().unwrap(),
        vec![(2, true)],
        "own write filtered, the fork's arrives with its provenance"
    );
}

#[backends(all)]
fn a_single_key_can_be_watched(backend: Backend) {
    let (_s, cfg) = cfg(backend);
    let seen = Arc::new(Mutex::new(0usize));
    let cap = seen.clone();

    let _sub = cfg
        .items()
        .subscription_with()
        .key("watched".to_string())
        .register(move |_: &MapChange<String, u64>| {
            *cap.lock().unwrap() += 1;
        });

    cfg.items().insert("watched".into(), &1).unwrap();
    cfg.items().insert("ignored".into(), &1).unwrap();

    assert_eq!(*seen.lock().unwrap(), 1);
}

#[backends(all)]
fn external_on_a_map_filters_updates_only(backend: Backend) {
    let (_s, cfg) = cfg(backend);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let cap = seen.clone();

    let _sub = cfg.items().subscription_with().external().register(
        move |change: &MapChange<String, u64>| {
            cap.lock().unwrap().push(match change {
                MapChange::Insert { .. } => "insert",
                MapChange::Update { .. } => "update",
                MapChange::Remove { .. } => "remove",
                MapChange::Clear { .. } => "clear",
            });
        },
    );

    cfg.items().insert("a".into(), &1).unwrap();
    cfg.items().update("a", &2).unwrap();
    cfg.items().remove("a").unwrap();

    assert_eq!(*seen.lock().unwrap(), vec!["insert", "remove"]);
}

#[backends(all)]
fn external_on_a_field_filters_everything_of_its_own(backend: Backend) {
    let (_s, cfg) = cfg(backend);
    let seen = Arc::new(Mutex::new(0usize));
    let cap = seen.clone();

    let _sub = cfg
        .counter()
        .subscription_with()
        .external()
        .register(move |_: &u64| *cap.lock().unwrap() += 1);

    cfg.counter().set(1).unwrap();
    cfg.counter().set(2).unwrap();

    assert_eq!(*seen.lock().unwrap(), 0);
}

mod stream {
    use super::*;
    use futures::StreamExt;
    use futures::executor::block_on;

    #[backends(all)]
    fn yields_each_change_in_order(backend: Backend) {
        let (_s, cfg) = cfg(backend);
        let mut changes = cfg.counter().subscription_with().stream();

        for n in 1..=3 {
            cfg.counter().set(n).unwrap();
        }

        let got: Vec<u64> = block_on(async { changes.by_ref().take(3).collect().await });
        assert_eq!(got, vec![1, 2, 3]);
    }

    #[backends(all)]
    fn a_write_from_another_thread_arrives(backend: Backend) {
        let (_s, cfg) = cfg(backend);
        let mut changes = cfg.counter().subscription_with().stream();

        let writer = cfg.counter();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            writer.set(99).unwrap();
        });

        let got = block_on(changes.next());
        handle.join().unwrap();

        assert_eq!(got, Some(99));
    }

    #[backends(all)]
    fn external_still_filters(backend: Backend) {
        let (_s, cfg) = cfg(backend);
        let field = cfg.counter();
        let other = field.fork();

        let mut changes = field.subscription_with().external().stream();

        field.set(1).unwrap();
        other.set(2).unwrap();

        assert_eq!(block_on(changes.next()), Some(2));
    }

    /// A stream yields what arrives after it exists, and nothing a stream
    /// before it queued. That the drop also released the subscription is not
    /// shown here and cannot be from outside: a dropped stream is not
    /// observable, and nothing exposes how many subscribers a signal has.
    #[backends(all)]
    fn a_stream_starts_empty_and_does_not_inherit_a_dropped_one(backend: Backend) {
        let (_s, cfg) = cfg(backend);
        let counter = cfg.counter();

        let changes = counter.subscription_with().stream();
        counter.set(1).unwrap();
        drop(changes);

        let mut fresh = counter.subscription_with().stream();
        counter.set(2).unwrap();

        assert_eq!(
            block_on(fresh.next()),
            Some(2),
            "the dropped stream's queue must not resurface"
        );
    }

    #[backends(all)]
    fn map_changes_stream_too(backend: Backend) {
        let (_s, cfg) = cfg(backend);
        let mut changes = cfg.items().subscription_with().stream();

        cfg.items().insert("a".into(), &1).unwrap();

        let got = block_on(changes.next()).unwrap();
        assert!(matches!(got, MapChange::Insert { .. }));
    }
}
