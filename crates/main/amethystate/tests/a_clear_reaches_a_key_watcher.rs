use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[backends(all)]
fn a_subscriber_on_one_key_hears_the_map_being_cleared(backend: Backend) {
    let path = TempPath::new("clear_reaches_key");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let widths = store.kv().map::<String, u64>("columns").unwrap();

    widths.insert("cpu".into(), &120u64).unwrap();
    widths.insert("mem".into(), &80u64).unwrap();

    let heard = Arc::new(AtomicUsize::new(0));
    let counting = heard.clone();
    let _sub = widths.subscribe_key("cpu".to_string(), move |_| {
        counting.fetch_add(1, Ordering::SeqCst);
    });

    widths.clear().unwrap();

    assert_eq!(widths.get("cpu"), None, "on {}", backend.extension());
    assert_eq!(
        heard.load(Ordering::SeqCst),
        1,
        "the key went with the clear and its watcher was not told, on {}",
        backend.extension()
    );
}

#[backends(all)]
fn an_entry_cell_empties_with_the_map(backend: Backend) {
    let path = TempPath::new("clear_reaches_cell");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let widths = store.kv().map::<String, u64>("columns").unwrap();

    widths.insert("cpu".into(), &120u64).unwrap();
    let cpu = widths.entry_cell("cpu".to_string());
    assert_eq!(cpu.get(), Some(120));

    widths.clear().unwrap();

    assert_eq!(widths.get("cpu"), None, "on {}", backend.extension());
    assert_eq!(
        cpu.get(),
        None,
        "the entry is gone and the cell still reports it, on {}",
        backend.extension()
    );
}
