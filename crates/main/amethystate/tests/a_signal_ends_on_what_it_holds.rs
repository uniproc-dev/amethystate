#![cfg(any(feature = "json", feature = "toml", feature = "ron"))]

use amethystate::Field;
use amethystate::store::builder::StoreBuilder;
use amethystate::store::field_with_path;
use amethystate_core::test_utils::TempPath;
use std::sync::{Arc, Barrier, Mutex};
use uuid::Uuid;

mod common;

const ROUNDS: usize = 300;

fn raced<F>(each: F)
where
    F: Fn(u64) + Send + Sync,
{
    let gate = Arc::new(Barrier::new(2));
    std::thread::scope(|scope| {
        for value in [1u64, 2u64] {
            let gate = Arc::clone(&gate);
            let each = &each;
            scope.spawn(move || {
                gate.wait();
                each(value);
            });
        }
    });
}

#[test]
#[ignore = "pins a way a signal ends on a value it no longer holds, which it still does - the store and the emit are two steps"]
fn a_volatile_field_and_the_cell_viewing_it_agree_after_a_race() {
    for round in 0..ROUNDS {
        let width = Field::<u64>::new_volatile(["ui", "width"], 0);
        let view = width.cell();

        raced(|value| width.set(value).unwrap());

        assert_eq!(
            view.get(),
            Some(width.get()),
            "round {round}: the cell views the field, so it cannot hold another value"
        );
    }
}

#[test]
#[ignore = "pins a way a signal ends on a value it no longer holds, which it still does - the store and the emit are two steps"]
fn a_volatile_field_never_ends_on_a_value_it_no_longer_holds() {
    for round in 0..ROUNDS {
        let width = Field::<u64>::new_volatile(["ui", "width"], 0);

        let last = Arc::new(Mutex::new(0u64));
        let sink = Arc::clone(&last);
        let _sub = width.subscribe(move |v: &u64| *sink.lock().unwrap() = *v);

        raced(|value| width.set(value).unwrap());

        assert_eq!(
            *last.lock().unwrap(),
            width.get(),
            "round {round}: the last value a subscriber was told is not the one the field holds"
        );
    }
}

#[test]
#[ignore = "pins a way a signal ends on a value it no longer holds, which it still does - the store settles the order under its lock and the emit leaves it"]
fn a_stored_field_agrees_with_the_store_after_a_race() {
    let path = TempPath::new("field_race");
    let store = StoreBuilder::new(path.path())
        .backend(common::text_backend())
        .build()
        .unwrap();

    for round in 0..ROUNDS {
        let at = format!("w{round}");
        let width = field_with_path::<u64>(&store, ["racers", &at], 0, Uuid::new_v4()).unwrap();

        raced(|value| width.set(value).unwrap());

        let stored: Option<u64> = store.get(["racers", &at]).unwrap();
        assert_eq!(
            Some(width.get()),
            stored,
            "round {round}: the field is a projection of the store, and the two disagree"
        );
    }
}
