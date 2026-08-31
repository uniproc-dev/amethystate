use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, Store};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::mpsc;
use std::time::Duration;

fn open(backend: Backend, tag: &str) -> (Store, TempPath, ReactiveMap<String, u64>) {
    let path = TempPath::new(tag);
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();
    let widths = store.kv().map::<String, u64>("columns").unwrap();
    (store, path, widths)
}

#[backends(all)]
fn a_write_during_a_walk_lands_and_the_walk_keeps_its_own_version(backend: Backend) {
    let (report, outcome) = mpsc::channel();

    std::thread::spawn(move || {
        let (_store, _path, widths) = open(backend, "walk_write_same_thread");
        widths.insert("cpu".into(), &120).unwrap();
        widths.insert("mem".into(), &80).unwrap();

        let mut walked = Vec::new();
        for (key, width) in widths.entries() {
            widths.insert("disk".into(), &60).unwrap();
            walked.push((key, width));
        }

        let _ = report.send((walked, widths.keys().collect::<Vec<_>>()));
    });

    match outcome.recv_timeout(Duration::from_secs(30)) {
        Ok((walked, after)) => {
            assert_eq!(
                walked,
                vec![("cpu".to_string(), 120), ("mem".to_string(), 80)]
            );
            assert_eq!(after, ["cpu", "disk", "mem"]);
        }
        Err(_) => panic!("the write inside the walk never returned"),
    }
}

#[backends(all)]
fn a_write_from_another_thread_does_not_wait_for_the_walk(backend: Backend) {
    let (_store, _path, widths) = open(backend, "walk_write_other_thread");

    widths.insert("cpu".into(), &120).unwrap();
    widths.insert("mem".into(), &80).unwrap();

    let walk = widths.entries();

    let (report, outcome) = mpsc::channel();
    let writer = widths.clone();
    std::thread::spawn(move || {
        let _ = report.send(writer.remove("cpu").is_ok());
    });

    match outcome.recv_timeout(Duration::from_secs(30)) {
        Ok(landed) => assert!(landed, "the other thread's write failed"),
        Err(_) => panic!("the other thread's write waited for a walk that holds no lock"),
    }

    assert_eq!(walk.count(), 2);
    assert_eq!(widths.get("cpu"), None);
    assert_eq!(widths.len(), 1);
}

#[backends(all)]
fn a_walk_that_removes_every_key_offers_each_one_once(backend: Backend) {
    let (_store, _path, widths) = open(backend, "walk_removes_all");

    widths.insert("cpu".into(), &120).unwrap();
    widths.insert("disk".into(), &60).unwrap();
    widths.insert("mem".into(), &80).unwrap();

    let mut walked = Vec::new();
    for (key, width) in widths.entries() {
        widths.remove(&key).unwrap();
        walked.push((key, width));
    }

    assert_eq!(
        walked,
        vec![
            ("cpu".to_string(), 120),
            ("disk".to_string(), 60),
            ("mem".to_string(), 80)
        ]
    );
    assert!(widths.is_empty());
}

#[backends(all)]
fn a_view_held_across_a_write_keeps_what_it_was_given(backend: Backend) {
    let (_store, _path, widths) = open(backend, "walk_view_pinned");

    widths.insert("cpu".into(), &120).unwrap();
    widths.insert("mem".into(), &80).unwrap();

    let held = widths.view();

    widths.insert("disk".into(), &60).unwrap();
    widths.remove("cpu").unwrap();

    assert_eq!(held.len(), 2);
    assert_eq!(
        held.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>(),
        ["cpu", "mem"]
    );
    assert_eq!(widths.keys().collect::<Vec<_>>(), ["disk", "mem"]);
}
