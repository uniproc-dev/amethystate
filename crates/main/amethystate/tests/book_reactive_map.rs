use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn open(
    backend: Backend,
    tag: &str,
) -> Result<(amethystate::Store, TempPath), Box<dyn Error + Send + Sync>> {
    let path = TempPath::new(tag);
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    Ok((store, path))
}

#[backends(all)]
fn reading_a_map(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_map_read")?;
    let widths = store.kv().map::<String, u64>("columns")?;
    widths.insert("cpu".to_string(), &120)?;
    widths.insert("mem".to_string(), &80)?;

    //@show reading a map
    let one: Option<u64> = widths.get("cpu");

    if let Some(width) = widths.get("cpu") {
        println!("cpu is {width}");
    }

    let there = widths.contains_key("mem");
    let how_many = widths.len();

    for key in widths.keys() {
        println!("{key}");
    }

    for (key, value) in widths.entries() {
        println!("{key}: {value}");
    }
    //@show-end

    assert_eq!(one, Some(120));
    assert!(there);
    assert_eq!(how_many, 2);

    Ok(())
}

#[backends(all)]
fn looking_without_taking(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_map_view")?;
    let widths = store.kv().map::<String, u64>("columns")?;
    widths.insert("cpu".to_string(), &120)?;
    widths.insert("mem".to_string(), &80)?;

    //@show looking at a map without cloning it
    let held = widths.view();
    let total: u64 = held.iter().map(|(_, width)| width).sum();

    let mut widest = 0;
    for (_, width) in &widths.view() {
        widest = widest.max(*width);
    }
    //@show-end

    assert_eq!(total, 200);
    assert_eq!(widest, 120);

    Ok(())
}

#[backends(all)]
fn walking_a_map_and_writing_to_it(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_map_walk_write")?;
    let widths = store.kv().map::<String, u64>("columns")?;
    widths.insert("cpu".to_string(), &120)?;
    widths.insert("mem".to_string(), &80)?;

    //@show walking a map and writing to it
    let mut walked = Vec::new();

    for (key, width) in widths.entries() {
        widths.remove(&key)?;
        walked.push((key, width));
    }

    assert_eq!(walked, [("cpu".to_string(), 120), ("mem".to_string(), 80)]);
    assert!(widths.is_empty());
    //@show-end

    Ok(())
}

#[backends(all)]
fn writing_to_a_map(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_map_write")?;
    let widths = store.kv().map::<String, u64>("columns")?;

    //@show adding, changing and removing an entry
    widths.insert("cpu".to_string(), &120)?;

    widths.update("cpu", &200)?;
    widths.modify("cpu", |width| *width += 10)?;

    let absent = widths.update("gpu", &90);

    widths.remove("cpu")?;
    widths.clear()?;
    //@show-end

    assert!(absent.is_err(), "update writes a key that is already there");
    assert!(widths.is_empty());

    Ok(())
}

#[backends(all)]
fn the_order_is_the_stores(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_map_order")?;
    let counts = store.kv().map::<String, u64>("counts")?;

    //@show the order entries come back in
    counts.insert("9".to_string(), &1)?;
    counts.insert("10".to_string(), &1)?;
    counts.insert("a.b".to_string(), &1)?;
    counts.insert("a1b".to_string(), &1)?;

    let order: Vec<String> = counts.keys().collect();
    assert_eq!(order, ["10", "9", "a1b", "a.b"]);
    //@show-end

    Ok(())
}

#[backends(all)]
fn hearing_about_a_change(backend: Backend) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (store, _path) = open(backend, "book_map_subs")?;
    let widths = store.kv().map::<String, u64>("columns")?;

    let anything = Arc::new(AtomicUsize::new(0));
    let one_key = Arc::new(AtomicUsize::new(0));
    let any_seen = Arc::clone(&anything);
    let key_seen = Arc::clone(&one_key);

    let _counting_all = widths.subscribe_any(move |_| {
        any_seen.fetch_add(1, Ordering::Release);
    });
    let _counting_cpu = widths.subscribe_key("cpu".to_string(), move |_| {
        key_seen.fetch_add(1, Ordering::Release);
    });

    //@show hearing about a change
    let _all = widths.subscribe_any(|change| {
        println!("{change:?}");
    });

    let _cpu = widths.subscribe_key("cpu".to_string(), |change| {
        println!("cpu: {change:?}");
    });

    widths.insert("cpu".to_string(), &120)?;
    widths.insert("mem".to_string(), &80)?;
    //@show-end

    assert_eq!(anything.load(Ordering::Acquire), 2);
    assert_eq!(one_key.load(Ordering::Acquire), 1);

    Ok(())
}
