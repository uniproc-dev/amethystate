use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeType, ReactiveMap, amethystate};
use amethystate_core::test_utils::unique_path;
use amethystate_test_macros::backends;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, AmeType)]
pub struct Limits {
    pub warning: u64,
}

#[amethystate(prefix = "dbg")]
pub struct Settings {
    #[amestate(default = 8080)]
    pub port: u16,

    #[amestate(default = "dark".to_string())]
    pub theme: String,

    #[amestate(default = { "cpu": Limits { warning: 70 } })]
    pub limits: ReactiveMap<String, Limits>,
}

/// Deliberately not `Debug`: the framework must not demand it.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, AmeType)]
pub struct Opaque {
    pub inner: u8,
}

#[amethystate(prefix = "dbg_opaque")]
pub struct HasOpaque {
    #[amestate(default = 1)]
    pub port: u16,

    #[amestate(default = Opaque { inner: 7 })]
    pub opaque: Opaque,
}

fn assert_debug<T: std::fmt::Debug>() {}

#[backends(all)]
fn a_field_type_need_not_be_printable(backend: Backend) {
    let store = StoreBuilder::new(unique_path("dbg_opaque"))
        .backend(backend)
        .build()
        .unwrap();
    let state = HasOpaque::new_with(&store).unwrap();

    assert_eq!(state.opaque().get().inner, 7);
    assert_eq!(state.port().get(), 1);
}

#[test]
fn a_printable_struct_still_gets_its_impl() {
    assert_debug::<Settings>();
}

fn settings(backend: Backend, tag: &str) -> Settings {
    let store = StoreBuilder::new(unique_path(tag))
        .backend(backend)
        .build()
        .unwrap();
    Settings::new_with(&store).unwrap()
}

#[backends(all)]
fn a_field_shows_its_path_and_value(backend: Backend) {
    let state = settings(backend, "dbg_field");
    let shown = format!("{:?}", state.port());

    assert!(shown.contains("Field"), "{shown}");
    assert!(shown.contains("dbg.port"), "{shown}");
    assert!(shown.contains("8080"), "{shown}");
}

#[backends(all)]
fn a_field_shows_the_current_value_not_the_default(backend: Backend) {
    let state = settings(backend, "dbg_current");
    state.port().set(9090).unwrap();

    let shown = format!("{:?}", state.port());
    assert!(shown.contains("9090"), "{shown}");
    assert!(!shown.contains("8080"), "{shown}");
}

#[backends(all)]
fn a_state_struct_shows_every_field_by_name(backend: Backend) {
    let state = settings(backend, "dbg_struct");
    let shown = format!("{state:?}");

    assert!(shown.starts_with("Settings {"), "{shown}");
    for expected in ["port", "8080", "theme", "dark", "limits", "cpu", "70"] {
        assert!(shown.contains(expected), "missing {expected} in {shown}");
    }
}

#[backends(all)]
fn the_instance_id_stays_out_of_the_output(backend: Backend) {
    let state = settings(backend, "dbg_no_id");
    let shown = format!("{state:?}");

    assert!(
        !shown.contains("__amethystate_instance_id"),
        "internals leaked: {shown}"
    );
}

#[amethystate(prefix = "dbg_p_opaque", mode = "persistent")]
pub struct PersistentOpaque {
    #[amestate(default = Opaque { inner: 3 })]
    pub opaque: Opaque,
}

#[derive(Debug)]
#[amethystate(prefix = "dbg_p_shown", mode = "persistent")]
pub struct PersistentShown {
    #[amestate(default = 5)]
    pub port: u16,
}

#[backends(all)]
fn a_persistent_struct_needs_no_debug_either(backend: Backend) {
    let store = StoreBuilder::new(unique_path("dbg_p_opaque"))
        .backend(backend)
        .build()
        .unwrap();
    let state = PersistentOpaque::load_with(&store).unwrap();

    assert_eq!(state.opaque.inner, 3);
}

#[backends(all)]
fn a_persistent_struct_prints_when_asked(backend: Backend) {
    let store = StoreBuilder::new(unique_path("dbg_p_shown"))
        .backend(backend)
        .build()
        .unwrap();
    let state = PersistentShown::load_with(&store).unwrap();

    let shown = format!("{state:?}");
    assert!(shown.contains("PersistentShown"), "{shown}");
    assert!(shown.contains("5"), "{shown}");
}
