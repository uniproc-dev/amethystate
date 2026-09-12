use amethystate::amethystate;
use amethystate::store::OpenStruct;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use std::sync::{Arc, Mutex};

mod common;
use common::{per_engine, shape};

#[amethystate(prefix = "typed")]
pub struct Typed {
    #[amestate(default = 8080u16)]
    pub port: u16,
}

fn store(backend: Backend) -> (TempPath, amethystate::Store) {
    let at = TempPath::new("kv");
    let store = StoreBuilder::new(&at).backend(backend).build().unwrap();
    (at, store)
}

#[backends(all)]
fn raw_round_trip(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    kv.set("theme", &"dark".to_string()).unwrap();
    assert_eq!(kv.get::<String>("theme").unwrap().as_deref(), Some("dark"));

    kv.remove("theme").unwrap();
    assert_eq!(kv.get::<String>("theme").unwrap(), None);
}

#[backends(all)]
fn a_cell_is_an_ordinary_reactive_cell(backend: Backend) {
    let (_at, store) = store(backend);

    //@show opening a cell without a schema
    let kv = store.kv();
    let width = kv.namespace("ui").cell("width", 800u32).unwrap();
    //@show-end

    assert_eq!(width.get(), Some(800), "seeded with the default");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let cap = Arc::clone(&seen);

    let _sub = width
        .subscription_with()
        .register(move |w: &Option<u32>| cap.lock().unwrap().push(w.unwrap()));

    width.set(1024).unwrap();

    assert_eq!(*seen.lock().unwrap(), vec![1024]);
    assert_eq!(store.get::<u32>(["ui", "width"]).unwrap(), Some(1024));
}

#[backends(all)]
fn a_map_takes_a_key_set_that_is_not_known_up_front(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    let flags = kv.map::<String, bool>("flags").unwrap();
    flags.insert("beta".into(), &true).unwrap();
    flags.insert("alpha".into(), &false).unwrap();

    assert_eq!(flags.keys().collect::<Vec<_>>(), ["alpha", "beta"]);
}

#[backends(all)]
fn keys_are_sorted_and_scoped_to_the_prefix(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    let ui = kv.namespace("ui");
    ui.set("zoom", &2u8).unwrap();
    ui.set("theme", &"dark".to_string()).unwrap();
    kv.namespace("net")
        .set("host", &"localhost".to_string())
        .unwrap();

    let keys = ui.keys().unwrap();
    assert_eq!(
        keys.iter().map(StorePath::to_string).collect::<Vec<_>>(),
        ["ui.theme", "ui.zoom"]
    );
}

#[backends(all)]
fn names_are_what_is_left_below_the_prefix(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    let ui = kv.namespace("ui");
    ui.set("zoom", &2u8).unwrap();
    ui.namespace("panel").set("left", &10u32).unwrap();
    kv.namespace("net")
        .set("host", &"localhost".to_string())
        .unwrap();

    let names = ui.names().unwrap();
    assert_eq!(
        names.iter().map(StorePath::to_string).collect::<Vec<_>>(),
        ["panel.left", "zoom"]
    );

    for name in &names {
        assert!(
            ui.get::<serde_json::Value>(&name.to_string()).is_ok(),
            "{backend:?}: a name this handed back is not one it takes: {name}"
        );
    }
}

#[backends(all)]
fn a_name_holding_the_separator_survives_the_prefix_coming_off(backend: Backend) {
    let (_at, store) = store(backend);

    let dotted = store.kv().namespace("a.b");
    dotted.set("c.d", &1u8).unwrap();

    let names = dotted.names().unwrap();
    assert_eq!(
        names.iter().map(StorePath::to_string).collect::<Vec<_>>(),
        ["c\\.d"],
        "{backend:?}: the prefix came off by its spelling rather than by its levels"
    );
    assert_eq!(names[0].len(), 1, "{backend:?}: one name became two levels");
}

#[backends(all)]
fn a_value_at_the_prefix_itself_has_no_name_below_it(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    kv.set("ui", &1u8).unwrap();
    kv.namespace("ui").set("zoom", &2u8).unwrap();

    let ui = kv.namespace("ui");

    assert_eq!(
        ui.names()
            .unwrap()
            .iter()
            .map(StorePath::to_string)
            .collect::<Vec<_>>(),
        ["zoom"],
        "{backend:?}: the value at the prefix came back as a name of no levels"
    );
    assert_eq!(
        ui.keys()
            .unwrap()
            .iter()
            .map(StorePath::to_string)
            .collect::<Vec<_>>(),
        ["ui", "ui.zoom"]
    );
}

#[backends(all)]
fn a_handle_with_no_prefix_names_what_it_keys(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    kv.namespace("ui").set("zoom", &2u8).unwrap();

    assert_eq!(kv.names().unwrap(), kv.keys().unwrap());
}

/// A declared path is the schema's. Writing there through Kv would not merely
/// store the wrong thing: the field's subscription fails to decode and keeps
/// its old value, and the next startup fails outright reading the path back.
///
/// The last of the four is the other direction - `typed` is not itself
/// declared, but a map there would take the level `typed.port` lives on.
#[backends(all)]
fn writing_into_a_declared_path_is_refused(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    let err = kv
        .namespace("typed")
        .set("port", &"not a number".to_string())
        .unwrap_err();
    insta::assert_snapshot!("kv_write_over_a_declared_field", err.to_string());

    let refused = [
        kv.namespace("typed")
            .cell("port", 1u16)
            .map(|_| ())
            .unwrap_err()
            .to_string(),
        kv.namespace("typed")
            .remove("port")
            .unwrap_err()
            .to_string(),
        kv.map::<String, u8>("typed")
            .map(|_| ())
            .unwrap_err()
            .to_string(),
    ];

    for (way, said) in ["cell", "remove", "map"].into_iter().zip(refused) {
        insta::assert_snapshot!(format!("kv_{way}_over_a_declared_field"), said);
    }
}

#[backends(all)]
fn a_path_next_to_a_declared_prefix_is_allowed(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    kv.namespace("typedish").set("port", &1u16).unwrap();
    assert_eq!(
        kv.namespace("typedish").get::<u16>("port").unwrap(),
        Some(1)
    );
}

/// A schema owns the paths it declared, not the prefix they sit under. Settings
/// are extended from outside all the time - a plugin, a theme, a person editing
/// the file - and none of that collides with a declared field.
#[backends(all)]
fn a_path_beside_a_declared_field_is_allowed(backend: Backend) {
    let (_at, store) = store(backend);
    let typed = store.kv().namespace("typed");

    typed.set("colour", &"blue".to_string()).unwrap();
    typed.namespace("myplugin").set("enabled", &true).unwrap();

    assert_eq!(
        typed.get::<String>("colour").unwrap().as_deref(),
        Some("blue")
    );
    assert_eq!(
        typed.namespace("myplugin").get::<bool>("enabled").unwrap(),
        Some(true)
    );

    assert!(
        typed.set("port", &1u16).is_err(),
        "the declared path itself is still owned"
    );
}

/// Nothing connects a path to a type the way a struct field does, so asking for
/// two types at one path is caught rather than returning garbage - by the read,
/// which is the only thing that can say what is actually stored there. The
/// report names the value, not just the two type names.
#[backends(all)]
fn the_same_path_cannot_be_two_types(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    let _width = kv.namespace("ui").cell("width", 800u32).unwrap();
    let err = kv.namespace("ui").cell("width", String::new()).unwrap_err();

    let OpenStruct::WillNotRead { why, .. } = &err else {
        panic!("{err}")
    };

    insta::assert_snapshot!(
        per_engine(backend, "kv_asked_for_a_second_type"),
        shape(why)
    );
}

#[backends(all)]
fn asking_for_the_same_path_and_type_twice_is_fine(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    let a = kv.namespace("ui").cell("width", 800u32).unwrap();
    let b = kv.namespace("ui").cell("width", 800u32).unwrap();

    a.set(42).unwrap();
    assert_eq!(b.get(), Some(42), "both are views on the same path");
}

/// The same, for a type that is not a primitive. Worth its own case because a
/// registry of type *names* got this wrong - `alloc::string::String` never
/// matched `String` - and refused the second ask for anything but a primitive.
#[backends(all)]
fn the_same_path_and_type_twice_is_fine_for_a_type_that_is_not_a_primitive(backend: Backend) {
    let (_at, store) = store(backend);
    let kv = store.kv();

    kv.cell("text", String::new()).unwrap();
    let again = kv.cell("text", String::new());
    assert!(
        again.is_ok(),
        "`String` was refused the second time: {:?}",
        again.err()
    );
}

#[backends(all)]
fn values_survive_a_reopen(backend: Backend) {
    let path = TempPath::new("kv_reopen");

    {
        let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
        store
            .kv()
            .namespace("ui")
            .cell("width", 800u32)
            .unwrap()
            .set(1280)
            .unwrap();
        store.save_now().unwrap();
    }

    let store = StoreBuilder::new(&path).backend(backend).build().unwrap();
    assert_eq!(
        store
            .kv()
            .namespace("ui")
            .cell("width", 800u32)
            .unwrap()
            .get(),
        Some(1280)
    );
}
