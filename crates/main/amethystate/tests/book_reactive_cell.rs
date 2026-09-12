use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{ReactiveCell, ReactiveMap, amethystate};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::collections::HashMap;

#[amethystate(prefix = "ui")]
pub struct Ui {
    #[amestate(default = 240u64)]
    pub sidebar_width: u64,

    #[amestate(default = { "cpu": 110u64 })]
    pub widths: ReactiveMap<String, u64>,
}

#[backends(all)]
fn four_things_erase_into_one_type(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_cell_sources");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    let state = Ui::new_with(&store)?;

    //@show four ways to reach a cell
    let width = state.sidebar_width().cell();
    let cpu_column = state.widths().entry_cell("cpu".to_string());
    let by_path = store.kv().cell("dragging", 0u64)?;
    let loose = ReactiveCell::new(0u64);

    let mut columns: HashMap<String, ReactiveCell<u64>> = HashMap::new();
    columns.insert("sidebar".to_string(), width);
    columns.insert("cpu".to_string(), cpu_column);
    columns.insert("dragging".to_string(), by_path);
    columns.insert("loose".to_string(), loose);
    //@show-end

    assert_eq!(columns["sidebar"].get(), Some(240));
    assert_eq!(columns["cpu"].get(), Some(110));
    assert_eq!(columns["dragging"].get(), Some(0));
    assert_eq!(columns["loose"].get(), Some(0));

    Ok(())
}

#[backends(all)]
fn a_cell_reads_writes_and_is_watched(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_cell_ops");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    let state = Ui::new_with(&store)?;
    let cell = state.sidebar_width().cell();

    //@show reading, writing and watching a cell
    let current = cell.get();

    let _sub = cell.subscribe(|width| println!("width -> {width:?}"));

    cell.set(200)?;
    cell.update(|width| width + 10)?;
    cell.modify(|width| *width += 10)?;
    //@show-end

    assert_eq!(current, Some(240));
    assert_eq!(cell.get(), Some(220));
    assert_eq!(state.sidebar_width().get(), 220);

    Ok(())
}

#[backends(all)]
fn an_entry_cell_is_empty_until_its_key_exists(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_cell_entry");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    let state = Ui::new_with(&store)?;

    //@show a cell onto a map entry
    state.widths().insert("cpu".to_string(), &120)?;

    let cpu = state.widths().entry_cell("cpu".to_string());
    let absent = state.widths().entry_cell("gpu".to_string());

    assert_eq!(cpu.get(), Some(120));
    assert_eq!(absent.get(), None);

    state.widths().remove("cpu")?;

    assert_eq!(cpu.get(), None);
    assert!(cpu.set(80).is_err());
    //@show-end

    Ok(())
}

#[backends(all)]
fn a_view_dies_with_its_source_and_an_owning_cell_does_not(backend: Backend) -> anyhow::Result<()> {
    let path = TempPath::new("book_cell_owning");
    let store = StoreBuilder::new(path.path()).backend(backend).build()?;
    let state = Ui::new_with(&store)?;

    //@show a view, and a cell that owns what feeds it
    let view = state.sidebar_width().cell();
    let owned = state.sidebar_width().into_cell();

    drop(state);
    //@show-end

    assert_eq!(
        view.get(),
        Some(240),
        "the owning cell below still holds the field both of them came from"
    );
    assert_eq!(owned.get(), Some(240));

    drop(owned);

    assert_eq!(
        view.get(),
        None,
        "with nothing holding the field, the view is empty"
    );

    Ok(())
}
