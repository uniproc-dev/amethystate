mod common;

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;

#[amethystate(prefix = "watched")]
pub struct Watched {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[amethystate(prefix = "held", mode = "persistent")]
pub struct Held {
    #[amestate(default = 800u32)]
    pub width: u32,

    #[amestate(default = "dark".to_string())]
    pub theme: String,
}

#[amethystate(prefix = "either", mode = "both")]
pub struct Either {
    #[amestate(default = 800u32)]
    pub width: u32,
}

#[backends(all)]
fn a_persistent_struct_loads_what_is_there_and_defaults_the_rest(backend: Backend) {
    let path = TempPath::new("shapes_persistent_load");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["held", "width"], &1920u32).unwrap();
    store.save_now().unwrap();

    let held = Held::load_with(&store).unwrap();

    assert_eq!(held.width, 1920, "what was stored");
    assert_eq!(
        held.theme, "dark",
        "and the declared default for what was not"
    );
}

#[backends(all)]
fn a_persistent_struct_writes_through_mutate(backend: Backend) {
    let path = TempPath::new("shapes_persistent_save");

    {
        let store = StoreBuilder::new(path.path())
            .backend(backend)
            .build()
            .unwrap();
        let mut held = Held::load_with(&store).unwrap();

        held.mutate(|it| it.width = 1280).unwrap();

        assert_eq!(
            store.get::<u32>(["held", "width"]).unwrap(),
            Some(1280),
            "`mutate` saves, so the store has it before anything is closed"
        );
        store.close().unwrap();
    }

    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    assert_eq!(Held::load_with(&store).unwrap().width, 1280);
}

#[backends(all)]
fn a_persistent_struct_reads_and_writes_through_deref(backend: Backend) {
    let path = TempPath::new("shapes_persistent_deref");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let mut held = Held::load_with(&store).unwrap();

    held.theme = "light".to_string();
    held.save().unwrap();

    assert_eq!(
        store.get::<String>(["held", "theme"]).unwrap().as_deref(),
        Some("light"),
        "`DerefMut` reaches the data, and `save` puts it where it goes"
    );
}

#[backends(all)]
fn both_gives_a_watching_half(backend: Backend) {
    let path = TempPath::new("shapes_both_watching");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let either = Either::new_with(&store).unwrap();
    either.width.set(1440).unwrap();
    store.save_now().unwrap();

    assert_eq!(either.width.get(), 1440);
    assert_eq!(store.get::<u32>(["either", "width"]).unwrap(), Some(1440));
}

#[backends(all)]
fn both_gives_a_loading_half_over_the_same_paths(backend: Backend) {
    let path = TempPath::new("shapes_both_loading");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let watching = Either::new_with(&store).unwrap();
    watching.width.set(1600).unwrap();
    store.save_now().unwrap();

    let loaded = Either::load_with(&store).unwrap();

    assert_eq!(
        loaded.width, 1600,
        "the two halves are over the same paths, so one sees what the other wrote"
    );
}

#[backends(all)]
fn both_writes_back_where_the_watching_half_reads(backend: Backend) {
    let path = TempPath::new("shapes_both_writeback");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let watching = Either::new_with(&store).unwrap();
    let mut loaded = Either::load_with(&store).unwrap();

    loaded.mutate(|it| it.width = 2048).unwrap();
    store.save_now().unwrap();

    assert_eq!(
        watching.width.get(),
        2048,
        "and the watching half is told about it"
    );
}

#[backends(all)]
fn the_watching_shape_is_still_what_it_was(backend: Backend) {
    let path = TempPath::new("shapes_reactive");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let watched = Watched::new_with(&store).unwrap();
    watched.width.set(1024).unwrap();
    store.save_now().unwrap();

    assert_eq!(store.get::<u32>(["watched", "width"]).unwrap(), Some(1024));
}
