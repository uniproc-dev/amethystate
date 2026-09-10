//! A map's keys are made at runtime, so nobody can declare them - which is why
//! it owns the whole space beneath it rather than the one level it uses.
//!
//! That is the difference from a struct. A struct's paths are all declared, so
//! the prefix it sits under stays open and an extension or a person editing the
//! file may put keys beside them. Under a map nothing is beside anything: every
//! name down there is a name the map might use.

mod common;

use amethystate::prelude::*;
use amethystate_core::test_utils::TempPath;
use amethystate_test_macros::backends;
use serde::{Deserialize, Serialize};

//@show a map and a leaf under one prefix
#[amethystate(prefix = "ui")]
pub struct Panels {
    #[amestate(default = {})]
    pub open: ReactiveMap<String, u32>,

    #[amestate(default = "dark".to_string())]
    pub theme: String,
}
//@show-end

/// Declares a leaf inside the map's space, which is the collision under test.
#[amethystate(prefix = "ui.open")]
pub struct Intruder {
    #[amestate(default = 0u32)]
    pub left: u32,
}

/// A value with depth of its own, to show that depth inside an entry is not
/// depth in the store - and one that holds itself, which nothing describing a
/// leaf's type could follow.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Branch {
    pub name: String,
    pub children: Vec<Branch>,
}

#[amethystate(prefix = "tree")]
pub struct Forest {
    #[amestate(default = {})]
    pub roots: ReactiveMap<String, Branch>,
}

fn opened(backend: Backend, at: &TempPath) -> Store {
    StoreBuilder::new(at.path())
        .backend(backend)
        .build()
        .unwrap()
}

#[backends(all)]
fn a_declared_leaf_inside_a_maps_space_is_refused(backend: Backend) {
    let at = TempPath::new("map_owns_leaf");
    let store = opened(backend, &at);

    Panels::new_with(&store).unwrap();

    let refused = Intruder::new_with(&store)
        .expect_err("`ui.open.left` is a name the map may use, so it is not free");

    let rendered = format!("{refused:?}");
    assert!(
        rendered.contains("ui.open"),
        "and the refusal names the claim it ran into: {rendered}"
    );
}

#[backends(all)]
fn it_is_refused_in_the_other_order_too(backend: Backend) {
    let at = TempPath::new("map_owns_order");
    let store = opened(backend, &at);

    Intruder::new_with(&store).unwrap();

    Panels::new_with(&store)
        .expect_err("a map cannot open over a leaf already declared inside its space");
}

#[backends(all)]
fn a_field_beside_the_map_is_left_alone(backend: Backend) {
    let at = TempPath::new("map_owns_sibling");
    let store = opened(backend, &at);

    let panels = Panels::new_with(&store).unwrap();

    panels.theme.set("light".to_string()).unwrap();
    panels.open.insert("left".to_string(), &240).unwrap();

    assert_eq!(
        panels.theme.get(),
        "light",
        "`ui.theme` is not under `ui.open`"
    );
}

#[backends(all)]
fn a_recursive_value_is_one_key_however_deep_it_goes(backend: Backend) {
    let at = TempPath::new("map_owns_recursive");
    let store = opened(backend, &at);

    let forest = Forest::new_with(&store).unwrap();

    let deep = Branch {
        name: "a".to_string(),
        children: vec![
            Branch {
                name: "b".to_string(),
                children: vec![Branch {
                    name: "b.deeper".to_string(),
                    children: vec![],
                }],
            },
            Branch {
                name: "c".to_string(),
                children: vec![],
            },
        ],
    };

    forest.roots.insert("one".to_string(), &deep).unwrap();
    store.save_now().unwrap();

    let under: Vec<String> = store
        .scan_keys(amethystate::store::to_path(["tree", "roots"]).unwrap())
        .unwrap()
        .into_iter()
        .map(|key| key.to_string())
        .collect();

    assert_eq!(
        under,
        ["tree.roots.one"],
        "the value is one stored thing, and its own depth is not the store's"
    );

    assert_eq!(
        forest.roots.get("one"),
        Some(deep),
        "and it comes back as it went in"
    );
}

#[backends(all)]
fn a_hand_written_key_under_a_map_is_refused(backend: Backend) {
    let at = TempPath::new("map_owns_by_hand");
    let store = opened(backend, &at);

    Panels::new_with(&store).unwrap();

    let refused = store
        .kv()
        .namespace("ui")
        .namespace("open")
        .set("left", &240u32)
        .expect_err("under a map there is no room beside the entries");

    let rendered = format!("{refused:?}");
    assert!(
        rendered.contains("ui.open"),
        "and it says whose space it is: {rendered}"
    );
}

#[backends(all)]
fn a_hand_written_key_beside_a_struct_is_allowed(backend: Backend) {
    let at = TempPath::new("map_owns_contrast");
    let store = opened(backend, &at);

    Panels::new_with(&store).unwrap();

    store
        .kv()
        .namespace("ui")
        .set("myplugin", &1u32)
        .expect("a struct's prefix stays open, which is the whole contrast");
}
