use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveMap, amethystate};

mod common;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "app")]
pub struct TableConfig {
    #[amestate(default = 110u64)]
    pub default_width: u64,

    #[amestate(default = {
        "name": 280u64,
        "cpu": 80u64,
    })]
    pub widths: ReactiveMap<String, u64>,
}

#[backends(all)]
fn entry_cell_reads_existing_value(backend: Backend) {
    let path = TempPath::new("entry_existing");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let config = TableConfig::new_with(&store).unwrap();

    let entry = config.widths().entry_cell("name".to_string());
    assert_eq!(entry.get(), Some(280));
}

/// A cell views an entry; it does not stand in for one. An absent key reads
/// empty, and writing through the view fails rather than quietly creating what
/// the map was never asked to hold.
#[backends(all)]
fn entry_cell_on_a_missing_key_is_empty_and_refuses_writes(backend: Backend) {
    let path = TempPath::new("entry_default");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let config = TableConfig::new_with(&store).unwrap();

    let entry = config.widths().entry_cell("disk".to_string());

    assert_eq!(entry.get(), None);

    let err = entry.set(110).unwrap_err();
    insta::assert_snapshot!("write_to_an_absent_key", err.to_string());

    assert_eq!(config.widths().get("disk"), None);
}

#[backends(all)]
fn write_lands_in_store(backend: Backend) {
    let path = TempPath::new("entry_write");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let config = TableConfig::new_with(&store).unwrap();

    let entry = config.widths().entry_cell("cpu".to_string());
    entry.set(144).unwrap();

    assert_eq!(config.widths().get("cpu"), Some(144));
    assert_eq!(entry.get(), Some(144));
}

#[backends(all)]
fn external_map_write_lands_in_cell(backend: Backend) {
    let path = TempPath::new("entry_external");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let config = TableConfig::new_with(&store).unwrap();

    let entry = config.widths().entry_cell("cpu".to_string());
    config.widths().update("cpu", &99).unwrap();

    assert_eq!(entry.get(), Some(99));
}

/// Removing the key empties the view rather than putting a stand-in value
/// there, and the view will not resurrect it - that is `insert`'s job.
#[backends(all)]
fn removed_key_empties_the_cell(backend: Backend) {
    let path = TempPath::new("entry_remove");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let config = TableConfig::new_with(&store).unwrap();

    let entry = config.widths().entry_cell("cpu".to_string());
    assert_eq!(entry.get(), Some(80));

    config.widths().remove("cpu").unwrap();

    assert_eq!(entry.get(), None);

    let err = entry.set(110).unwrap_err();
    insta::assert_snapshot!("write_to_a_removed_key", err.to_string());

    assert_eq!(config.widths().get("cpu"), None);
}

/// Two independent cells on the same key - a table column and, say, a settings
/// editor. Each side must see the other's writes, and each write must raise a
/// subscriber exactly once.
///
/// This is the case the whole refactor is about. The old entry signal wrote its
/// own cache *and* took the change again when the store echoed it back, so a
/// local write fired twice; only a remote write fired once. With the map
/// subscription as the sole writer, both directions cost one fire.
#[backends(all)]
fn two_cells_on_one_key_fire_once_per_write(backend: Backend) {
    let path = TempPath::new("entry_no_echo");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let config = TableConfig::new_with(&store).unwrap();

    let a = config.widths().entry_cell("cpu".to_string());
    let b = config.widths().entry_cell("cpu".to_string());

    let fires = Arc::new(AtomicUsize::new(0));
    let fires_clone = fires.clone();
    let _watch = a.subscribe(move |_: &Option<u64>| {
        fires_clone.fetch_add(1, Ordering::SeqCst);
    });

    b.set(200).unwrap();

    assert_eq!(a.get(), Some(200), "b's write must reach a");
    assert_eq!(config.widths().get("cpu"), Some(200));
    assert_eq!(
        fires.load(Ordering::SeqCst),
        1,
        "a remote write must fire a exactly once"
    );

    a.set(300).unwrap();

    assert_eq!(b.get(), Some(300), "a's write must reach b");
    assert_eq!(
        fires.load(Ordering::SeqCst),
        2,
        "a local write must fire once too, not twice"
    );
}

/// A rejected write must say so and leave the cell reporting what is stored.
/// The old entry signal swallowed the error and kept the value it had already
/// written into its own cache, so the cell quietly disagreed with the store.
#[backends(all)]
fn rejected_write_reports_and_leaves_the_cell_alone(backend: Backend) {
    let path = TempPath::new("entry_rejected");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let config = TableConfig::new_with(&store).unwrap();

    let entry = config.widths().entry_cell("cpu".to_string());
    let _guard = config.widths().intercept(|_change| None);

    let result = entry.set(999);

    let err = result.unwrap_err();
    insta::assert_snapshot!("write_an_interceptor_refused", err.to_string());
    assert_eq!(
        entry.get(),
        Some(80),
        "cell must not hold a value the map refused"
    );
    assert_eq!(config.widths().get("cpu"), Some(80));
}

/// The cell holds the map weakly, and a handle is not the map: dropping the
/// clone it was made from changes nothing while the struct still holds one.
#[backends(all)]
fn a_cell_survives_the_handle_it_was_made_from(backend: Backend) {
    let path = TempPath::new("entry_keepalive");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let config = TableConfig::new_with(&store).unwrap();

    let entry = {
        let widths = config.widths();
        widths.entry_cell("cpu".to_string())
    };

    config.widths().update("cpu", &55).unwrap();

    assert_eq!(entry.get(), Some(55), "cache must still be fed");
}

/// And once the last handle is gone there is nothing left to view, so the cell
/// stops answering instead of reporting the last value it happened to see.
#[backends(all)]
fn a_cell_dies_with_the_map_it_views(backend: Backend) {
    let path = TempPath::new("entry_dead_map");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let entry = {
        let map = store.kv().map::<String, u64>("standalone").unwrap();
        map.insert("cpu".to_string(), &80).unwrap();
        map.entry_cell("cpu".to_string())
    };

    assert_eq!(entry.get(), None);

    let err = entry.set(1).unwrap_err();
    insta::assert_snapshot!("write_through_a_cell_whose_map_is_gone", err.to_string());
}

/// Integration: a write through the entry cell survives a store rebuild.
#[backends(all)]
fn entry_cell_write_persists_across_store_rebuild(backend: Backend) {
    let path = TempPath::new("entry_persist");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = TableConfig::new_with(&store).unwrap();
        config.widths().insert("memory".to_string(), &0).unwrap();
        let entry = config.widths().entry_cell("memory".to_string());
        entry.set(256).unwrap();
    }

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let config = TableConfig::new_with(&store).unwrap();
        assert_eq!(config.widths().get("memory"), Some(256));

        let entry = config.widths().entry_cell("memory".to_string());
        assert_eq!(entry.get(), Some(256));
    }
}

/// The point of the whole thing: a cell left behind in a UI must not keep the
/// database file open. redb refuses a second open of a live file, so a
/// successful reopen is the proof that the first one was really released.
#[backends(all)]
fn a_forgotten_cell_does_not_hold_the_store_open(backend: Backend) {
    let path = TempPath::new("entry_no_pin");

    let entry = {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let map = store.kv().map::<String, u64>("widths").unwrap();
        map.insert("cpu".to_string(), &80).unwrap();
        map.entry_cell("cpu".to_string())
    };

    let reopened = StoreBuilder::new(path.path()).backend(backend).build();
    assert!(
        reopened.is_ok(),
        "the cell outlived the store and kept the file open: {:?}",
        reopened.err()
    );

    assert_eq!(entry.get(), None);
}

/// Building a struct, handing one of its values away and dropping the struct is
/// an ordinary shape in UI code. A view would go empty there, so the owning
/// form exists and says as much at the call site.
#[backends(all)]
fn an_owning_cell_survives_the_struct_it_came_from(backend: Backend) {
    let path = TempPath::new("entry_owned");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let width = {
        let config = TableConfig::new_with(&store).unwrap();
        config.default_width().into_cell()
    };

    assert_eq!(width.get(), Some(110));
    width.set(140).unwrap();
    assert_eq!(
        store.get::<u64>(["app", "default_width"]).unwrap(),
        Some(140)
    );
}

#[backends(all)]
fn an_owning_entry_cell_survives_the_map_it_came_from(backend: Backend) {
    let path = TempPath::new("entry_owned_map");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();

    let cpu = {
        let map = store.kv().map::<String, u64>("widths").unwrap();
        map.insert("cpu".to_string(), &80).unwrap();
        map.into_entry_cell("cpu".to_string())
    };

    assert_eq!(cpu.get(), Some(80));
    cpu.set(96).unwrap();
    assert_eq!(cpu.get(), Some(96));
}
