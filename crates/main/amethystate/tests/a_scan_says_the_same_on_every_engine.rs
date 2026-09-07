use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::{StoreBackend, StorePath};
use amethystate::{ReactiveMap, Store};
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;
use std::collections::HashMap;

fn seeded() -> HashMap<String, u32> {
    let mut held = HashMap::new();
    held.insert("alpha".to_string(), 1);
    held
}

#[amethystate]
pub struct Part {
    #[amestate(default = 2u32)]
    pub inner: u32,
}

#[amethystate(prefix = "shape")]
pub struct Shape {
    #[amestate(default = 1u32)]
    pub leaf: u32,

    #[amestate(nested)]
    pub part: Part,

    #[amestate(default = seeded())]
    pub items: ReactiveMap<String, u32>,
}

fn listed(store: &Store) -> Vec<String> {
    let mut found: Vec<String> = StoreBackend::scan_keys(store, &StorePath::segment("shape"))
        .unwrap()
        .into_iter()
        .map(|at| at.as_str().to_string())
        .collect();
    found.sort();
    found
}

#[backends(all)]
fn a_declared_shape_lists_its_places(backend: Backend) {
    let path = TempPath::new("scan_same_declared");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let shape = Shape::new_with(&store).unwrap();

    assert_eq!(
        listed(&store),
        ["shape.items.alpha", "shape.leaf", "shape.part.inner"],
        "on {}",
        backend.extension()
    );

    shape.items().insert("beta".into(), &2u32).unwrap();

    assert_eq!(
        listed(&store),
        [
            "shape.items.alpha",
            "shape.items.beta",
            "shape.leaf",
            "shape.part.inner"
        ],
        "on {}",
        backend.extension()
    );
}

#[backends(all)]
fn a_map_emptied_entry_by_entry_leaves_no_key_of_its_own(backend: Backend) {
    let path = TempPath::new("scan_same_emptied");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let shape = Shape::new_with(&store).unwrap();

    shape.items().remove("alpha").unwrap();

    assert_eq!(
        listed(&store),
        ["shape.leaf", "shape.part.inner"],
        "the map's own level is not a key, and only a document engine has one to \
         report - on {}",
        backend.extension()
    );
}

#[backends(all)]
fn a_map_cleared_whole_leaves_no_key_of_its_own(backend: Backend) {
    let path = TempPath::new("scan_same_cleared");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let shape = Shape::new_with(&store).unwrap();

    shape.items().insert("beta".into(), &2u32).unwrap();
    shape.items().clear().unwrap();

    assert_eq!(
        listed(&store),
        ["shape.leaf", "shape.part.inner"],
        "on {}",
        backend.extension()
    );
}

#[backends(all)]
fn a_key_nothing_declares_is_listed_beside_the_declared_ones(backend: Backend) {
    let path = TempPath::new("scan_same_undeclared");
    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let _shape = Shape::new_with(&store).unwrap();

    store.kv().namespace("shape").set("beside", &7u32).unwrap();

    assert_eq!(
        listed(&store),
        [
            "shape.beside",
            "shape.items.alpha",
            "shape.leaf",
            "shape.part.inner"
        ],
        "on {}",
        backend.extension()
    );
}

#[backends(all)]
fn a_shape_that_survives_a_reopen_lists_the_same(backend: Backend) {
    let path = TempPath::new("scan_same_reopen");
    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        let shape = Shape::new_with(&store).unwrap();
        shape.items().remove("alpha").unwrap();
        store.save_now().unwrap();
    }

    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    let _shape = Shape::new_with(&store).unwrap();

    assert_eq!(
        listed(&store),
        ["shape.leaf", "shape.part.inner"],
        "on {}",
        backend.extension()
    );
}
