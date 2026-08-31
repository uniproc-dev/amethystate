use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

#[amethystate(prefix = "re")]
pub struct Cfg {
    #[amestate(default = 0u64)]
    pub counter: u64,

    #[amestate(default = {})]
    pub items: ReactiveMap<String, u64>,
}

fn within<F>(what: &str, body: F)
where
    F: FnOnce() + Send + 'static,
{
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        body();
        let _ = tx.send(());
    });

    if rx.recv_timeout(Duration::from_secs(5)).is_err() {
        panic!("{what} deadlocked");
    }
}

/// Reacting to a change by writing is ordinary, and the subscriber lists are
/// behind non-reentrant mutexes, so notifying while holding them hangs the
/// thread outright.
#[backends(all)]
fn a_map_subscriber_may_write_to_the_map_it_watches(backend: Backend) {
    within("map subscriber writing to its own map", move || {
        let path = unique_path("reentrancy_map");
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let cfg = Cfg::new_with(&store).unwrap();
        let items = cfg.items();

        let writer = items.clone();
        let _sub = items.subscribe_any(move |change| {
            if change.key().map(String::as_str) == Some("mirror") {
                return;
            }
            let _ = writer.insert("mirror".to_string(), &99);
        });

        items.insert("a".to_string(), &1).unwrap();

        // Without this the test says only that a synchronous call returned, and
        // an empty `notify` would satisfy it - the callback that was supposed
        // to deadlock simply never runs.
        assert_eq!(
            items.get("mirror"),
            Some(99),
            "the subscriber never ran, so nothing was reentered"
        );
    });
}

#[backends(all)]
fn a_keyed_subscriber_may_write_to_the_map_it_watches(backend: Backend) {
    within("keyed subscriber writing to its own map", move || {
        let path = unique_path("reentrancy_key");
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let cfg = Cfg::new_with(&store).unwrap();
        let items = cfg.items();

        let writer = items.clone();
        let _sub = items.subscribe_key("a".to_string(), move |_| {
            let _ = writer.insert("mirror".to_string(), &99);
        });

        items.insert("a".to_string(), &1).unwrap();

        assert_eq!(
            items.get("mirror"),
            Some(99),
            "the keyed subscriber never ran, so nothing was reentered"
        );
    });
}

#[backends(all)]
fn a_subscriber_may_add_another_subscription_while_being_notified(backend: Backend) {
    within("subscriber subscribing during a notification", move || {
        let path = unique_path("reentrancy_sub");
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let cfg = Cfg::new_with(&store).unwrap();
        let items = cfg.items();

        let nested = items.clone();
        let ran = Arc::new(AtomicUsize::new(0));
        let ran_in = ran.clone();
        let _sub = items.subscribe_any(move |_| {
            let extra = nested.subscribe_any(|_| {});
            drop(extra);
            ran_in.fetch_add(1, Ordering::SeqCst);
        });

        items.insert("a".to_string(), &1).unwrap();

        assert_eq!(
            ran.load(Ordering::SeqCst),
            1,
            "the subscriber never ran, so it never subscribed from inside a \
             notification and nothing was reentered"
        );
    });
}

/// A panicking callback used to poison the subscriber lists, after which every
/// later subscribe panicked and every later notify silently delivered nothing.
#[backends(all)]
fn a_panicking_subscriber_does_not_disable_the_map(backend: Backend) {
    let path = unique_path("reentrancy_panic");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let cfg = Cfg::new_with(&store).unwrap();
    let items = cfg.items();

    let boom = items.subscribe_any(|_| panic!("subscriber blew up"));

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = items.insert("a".to_string(), &1);
    }));
    std::panic::set_hook(previous);
    assert!(
        result.is_err(),
        "the panic should surface, not be swallowed"
    );
    drop(boom);

    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cap = seen.clone();
    let _sub = items.subscribe_any(move |_| {
        cap.fetch_add(1, Ordering::SeqCst);
    });

    items.insert("b".to_string(), &2).unwrap();

    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "notifications must survive an earlier panicking subscriber"
    );
}
