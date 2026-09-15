#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

const KEYS: u32 = 200;

fn opened(at: &Path) -> amethystate::Store {
    StoreBuilder::new(at)
        .backend(Backend::Json)
        .disk(|d| d.watch_every(Duration::from_millis(50)))
        .build()
        .unwrap()
}

#[test]
fn two_stores_opening_one_file_at_once_both_open() {
    for round in 0..20 {
        let at = TempPath::new("opening_at_once");
        drop(opened(at.path()));

        let together = Arc::new(Barrier::new(2));
        let openers: Vec<_> = (0..2)
            .map(|_| {
                let together = together.clone();
                let path = at.path().to_path_buf();
                thread::spawn(move || {
                    together.wait();
                    StoreBuilder::new(&path)
                        .backend(Backend::Json)
                        .build()
                        .map(drop)
                })
            })
            .collect();

        for opener in openers {
            let opened = opener.join().unwrap();
            assert!(opened.is_ok(), "round {round}: {opened:?}");
        }
    }
}

#[test]
fn a_store_opened_beside_another_leaves_what_the_other_wrote() {
    let at = TempPath::new("opened_beside");
    let writer = opened(at.path());
    let path = at.path().to_path_buf();

    let opening = thread::spawn(move || {
        for _ in 0..40 {
            drop(StoreBuilder::new(&path).backend(Backend::Json).build());
        }
    });

    for key in 0..KEYS {
        writer.kv().set(&format!("k{key}"), &key).unwrap();
        let _ = writer.save_now();
    }

    opening.join().unwrap();
    writer.save_now().unwrap();
    thread::sleep(Duration::from_millis(300));

    let forgotten: Vec<u32> = (0..KEYS)
        .filter(|key| writer.kv().get::<u32>(&format!("k{key}")).unwrap() != Some(*key))
        .collect();
    assert!(
        forgotten.is_empty(),
        "the writer no longer holds {forgotten:?}"
    );

    drop(writer);
    let reopened = opened(at.path());

    let lost: Vec<u32> = (0..KEYS)
        .filter(|key| reopened.kv().get::<u32>(&format!("k{key}")).unwrap() != Some(*key))
        .collect();
    assert!(lost.is_empty(), "the file no longer holds {lost:?}");
}
