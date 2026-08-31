//! What a namespace keeps when it is cleared.
//!
//! Settings are extended from outside: an extension writes its own keys, a
//! theme writes its own, a person edits the file. Clearing a namespace is how
//! those go away again, and it must not take the declared settings with them.

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_test_macros::backends;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::unique_path;

#[amethystate(prefix = "app")]
pub struct App {
    #[amestate(default = 1280u32)]
    pub width: u32,

    #[amestate(nested)]
    pub panel: Panel,

    #[amestate(default = { "left": 200u32 })]
    pub widths: amethystate::ReactiveMap<String, u32>,
}

#[amethystate]
pub struct Panel {
    #[amestate(default = true)]
    pub visible: bool,
}

fn store(backend: Backend) -> amethystate::Store {
    StoreBuilder::new(unique_path("kv_clear"))
        .backend(backend)
        .build()
        .unwrap()
}

fn at(joined: &str) -> StorePath {
    StorePath::parse_joined(joined).unwrap()
}

#[backends(all)]
fn clearing_takes_what_no_schema_declared_and_leaves_the_rest(backend: Backend) {
    let store = store(backend);
    let app = App::new_with(&store).unwrap();
    let kv = store.kv().namespace("app");

    kv.namespace("myplugin").set("enabled", &true).unwrap();
    kv.set("theme", &"dark".to_string()).unwrap();

    let cleared = kv.clear().unwrap();

    assert!(
        cleared
            .removed
            .iter()
            .any(|p| p.starts_with(&at("app.myplugin"))),
        "an undeclared subtree survived: {:?}",
        cleared.removed
    );
    assert!(
        cleared.removed.contains(&at("app.theme")),
        "an undeclared key survived: {:?}",
        cleared.removed
    );
    assert!(
        cleared.kept.contains(&at("app.width")),
        "a declared field was not reported as kept: {:?}",
        cleared.kept
    );

    assert_eq!(app.width().get(), 1280, "the declared value is untouched");
    assert_eq!(
        store.kv().namespace("app").get::<String>("theme").unwrap(),
        None
    );
    assert_eq!(
        store
            .kv()
            .namespace("app")
            .namespace("myplugin")
            .get::<bool>("enabled")
            .unwrap(),
        None
    );
}

/// A declared path under an undeclared level is descended into, not skipped -
/// otherwise an extension writing beside a nested struct's field would be
/// immortal.
#[backends(all)]
fn clearing_descends_past_a_level_that_holds_a_declared_path(backend: Backend) {
    let store = store(backend);
    let app = App::new_with(&store).unwrap();
    let panel = store.kv().namespace("app").namespace("panel");

    panel.set("colour", &"blue".to_string()).unwrap();

    let cleared = store.kv().namespace("app").clear().unwrap();

    assert!(
        cleared.removed.contains(&at("app.panel.colour")),
        "an undeclared key beside a nested field survived: {:?}",
        cleared.removed
    );
    assert!(
        cleared.kept.contains(&at("app.panel.visible")),
        "the nested field was not kept: {:?}",
        cleared.kept
    );
    assert!(
        app.panel().visible().get(),
        "the nested declared value is untouched"
    );
}

/// Clearing is not restoring defaults: a declared value that was changed keeps
/// its change.
#[backends(all)]
fn clearing_does_not_restore_a_declared_default(backend: Backend) {
    let store = store(backend);
    let app = App::new_with(&store).unwrap();
    app.width().set(640).unwrap();

    store.kv().namespace("app").clear().unwrap();

    assert_eq!(app.width().get(), 640);
    assert_eq!(store.get::<u32>(["app", "width"]).unwrap(), Some(640));
}

/// The other half. Dropping the values is not enough on its own: the marker
/// saying the namespace was seeded has to go with them, or the next
/// construction reads "already been here" and writes nothing, and the settings
/// are gone rather than reset.
#[backends(all)]
fn resetting_puts_the_declared_defaults_back_on_the_next_build(backend: Backend) {
    let path = unique_path("kv_reset");

    {
        let store = StoreBuilder::new(&path)
            .backend(backend)
            .build()
            .unwrap();
        let app = App::new_with(&store).unwrap();
        app.width().set(640).unwrap();
        app.panel().visible().set(false).unwrap();
        store.kv().namespace("app").set("theme", &1u8).unwrap();
        store.save_now().unwrap();
    }

    {
        let store = StoreBuilder::new(&path)
            .backend(backend)
            .build()
            .unwrap();
        let cleared = store.kv().namespace("app").reset_to_defaults().unwrap();
        assert!(!cleared.removed.is_empty(), "nothing was reset");
        store.save_now().unwrap();
    }

    let store = StoreBuilder::new(&path)
        .backend(backend)
        .build()
        .unwrap();
    let app = App::new_with(&store).unwrap();

    assert_eq!(app.width().get(), 1280, "the default did not come back");
    assert!(
        app.panel().visible().get(),
        "the nested default did not come back"
    );
    assert_eq!(
        store.kv().namespace("app").get::<u8>("theme").unwrap(),
        Some(1),
        "resetting the schema's paths took an undeclared one with it"
    );
    assert_eq!(
        app.widths().get("left"),
        Some(200),
        "the map's default did not come back"
    );
}
