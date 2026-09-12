use amethystate::ReactiveCell;
use std::sync::{Arc, Mutex};

#[test]
fn an_in_memory_cell_filters_the_writes_it_made_itself() {
    let mode = ReactiveCell::new(1u64);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let _sub = mode
        .subscription_with()
        .external()
        .register(move |v: &Option<u64>| sink.lock().unwrap().push(*v));

    mode.set(2).unwrap();

    assert_eq!(mode.get(), Some(2));
    assert_eq!(*seen.lock().unwrap(), Vec::<Option<u64>>::new());
}

#[test]
fn an_in_memory_cell_still_hears_its_own_writes_without_external() {
    let mode = ReactiveCell::new(1u64);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let _sub = mode
        .subscription_with()
        .register(move |v: &Option<u64>| sink.lock().unwrap().push(*v));

    mode.set(2).unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![Some(2)]);
}
