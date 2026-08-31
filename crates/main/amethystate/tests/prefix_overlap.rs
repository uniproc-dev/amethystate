use amethystate::store::StorageError;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::store::owners::Claimed;
use amethystate::{StorageResult, Store};
use amethystate_core::facts::all;
use amethystate_core::test_utils::TempPath;
use amethystate_macros::amethystate;
use amethystate_test_macros::backends;

mod common;
use common::shape;

#[amethystate(prefix = "coll", version = 1)]
pub struct Outer {
    #[amestate(key = "panels.left.visible", default = true)]
    pub left_panel_visible: bool,
}

#[amethystate(prefix = "coll.panels", version = 1)]
pub struct Panels {
    #[amestate(key = "left.visible", default = true)]
    pub left_visible: bool,
}

#[amethystate(prefix = "coll.panels.left", version = 1)]
pub struct Left {
    #[amestate(default = true)]
    pub visible: bool,
}

#[amethystate(prefix = "typed", version = 1)]
pub struct TypedOuter {
    #[amestate(key = "panels.left.visible", default = true)]
    pub left_panel_visible: bool,
}

#[amethystate(prefix = "typed.panels", version = 1)]
pub struct TypedPanels {
    #[amestate(key = "left.visible", default = 0u32)]
    pub left_visible: u32,
}

fn contested(
    backend: Backend,
    at: &str,
    first: fn(&Store) -> StorageResult<()>,
    second: fn(&Store) -> StorageResult<()>,
) -> String {
    let path = TempPath::new(at);
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    first(&store).expect("the first spelling is free to take it");
    let refused = second(&store).expect_err("the second wanted the same place");

    assert_eq!(
        refused.current_context(),
        &StorageError::Claimed,
        "refused, but not as a claim: {refused:?}"
    );

    let claims: Vec<&Claimed> = all::<Claimed, _>(&refused).collect();

    assert_eq!(claims.len(), 2, "the report names both: {refused:?}");
    for claim in &claims {
        assert_eq!(
            claim.path.as_str(),
            "coll.panels.left.visible",
            "the refusal is about the place all three spell: {refused:?}"
        );
    }
    assert_ne!(
        claims[0].by, claims[1].by,
        "and attributes it to two different schemas: {refused:?}"
    );

    shape(&refused)
}

#[backends(all)]
fn a_dotted_key_and_a_dotted_prefix_reach_one_place(backend: Backend) {
    insta::assert_snapshot!(
        "overlap_key_against_prefix",
        contested(
            backend,
            "overlap_key_prefix",
            |s| Outer::new_with(s).map(|_| ()),
            |s| Panels::new_with(s).map(|_| ()),
        )
    );
}

#[backends(all)]
fn a_dotted_key_and_a_prefix_all_the_way_down_reach_one_place(backend: Backend) {
    insta::assert_snapshot!(
        "overlap_key_against_deep_prefix",
        contested(
            backend,
            "overlap_key_deep",
            |s| Outer::new_with(s).map(|_| ()),
            |s| Left::new_with(s).map(|_| ()),
        )
    );
}

#[backends(all)]
fn two_prefixes_of_different_depth_reach_one_place(backend: Backend) {
    insta::assert_snapshot!(
        "overlap_prefix_against_deep_prefix",
        contested(
            backend,
            "overlap_prefix_deep",
            |s| Panels::new_with(s).map(|_| ()),
            |s| Left::new_with(s).map(|_| ()),
        )
    );
}

#[backends(all)]
fn one_claim_refuses_every_other_spelling_not_only_the_next(backend: Backend) {
    let path = TempPath::new("overlap_chain");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    Outer::new_with(&store).unwrap();

    for refused in [
        Panels::new_with(&store).map(|_| ()).unwrap_err(),
        Left::new_with(&store).map(|_| ()).unwrap_err(),
    ] {
        let by: Vec<&str> = all::<Claimed, _>(&refused).map(|claim| claim.by).collect();

        assert_eq!(by.len(), 2, "two claims, not a pile: {refused:?}");
        assert!(
            by.iter().any(|by| by.ends_with("Outer")),
            "the standing claim is still Outer's: {refused:?}"
        );
    }

    Outer::new_with(&store).expect("a refusal must not disturb the claim it refused for");
}

#[backends(all)]
fn the_chain_refuses_in_either_order(backend: Backend) {
    let path = TempPath::new("overlap_chain_reversed");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    Left::new_with(&store).unwrap();

    assert!(Panels::new_with(&store).is_err(), "the middle spelling");
    assert!(Outer::new_with(&store).is_err(), "the shallow spelling");

    Left::new_with(&store).expect("a refusal must not disturb the claim it refused for");
}

#[backends(all)]
fn a_prefix_is_refused_by_a_field_already_under_it(backend: Backend) {
    let path = TempPath::new("root_is_a_leaf_reversed");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    Branch::new_with(&store).unwrap();
    let refused = Root::new_with(&store).unwrap_err();

    let claims: Vec<&str> = all::<Claimed, _>(&refused)
        .map(|claim| claim.path.as_str())
        .collect();

    assert!(
        claims.contains(&"root.b") && claims.contains(&"root.b.x"),
        "both places are named whichever was claimed first: {refused:?}"
    );
}

#[backends(all)]
fn a_claim_outlives_the_handle_that_made_it(backend: Backend) {
    let path = TempPath::new("overlap_dropped");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    drop(Outer::new_with(&store).unwrap());

    assert!(
        Panels::new_with(&store).is_err(),
        "dropping the first struct must not free the place it claimed"
    );
}

#[backends(all)]
fn an_overlap_between_different_types_is_reported_as_an_overlap(backend: Backend) {
    let path = TempPath::new("prefix_overlap_typed");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let _outer = TypedOuter::new_with(&store).unwrap();
    let refused = TypedPanels::new_with(&store).unwrap_err();

    let rendered = format!("{refused:?}");
    assert!(
        rendered.contains("TypedOuter") && rendered.contains("TypedPanels"),
        "the report should name both schemas claiming `typed.panels.left.visible`, got: {rendered}"
    );
}

#[amethystate(prefix = "root", version = 1)]
pub struct Root {
    #[amestate(default = 1u32)]
    pub b: u32,
}

#[amethystate(prefix = "root.b", version = 1)]
pub struct Branch {
    #[amestate(default = 2u32)]
    pub x: u32,
}

#[backends(all)]
fn a_prefix_may_not_land_on_another_structs_field(backend: Backend) {
    let path = TempPath::new("root_is_a_leaf");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    let _root = Root::new_with(&store).unwrap();
    let refused = Branch::new_with(&store).unwrap_err();

    let claims: Vec<String> = all::<Claimed, _>(&refused)
        .map(|claim| claim.path.as_str().to_string())
        .collect();

    assert!(
        claims.iter().any(|p| p == "root.b") && claims.iter().any(|p| p == "root.b.x"),
        "the report names the leaf and the branch that wanted to sit under it: {claims:?}"
    );
}

#[backends(all)]
fn a_map_will_not_open_over_keys_deeper_than_its_entries(backend: Backend) {
    let path = TempPath::new("map_swallows_below");
    let store = StoreBuilder::new(path.path())
        .backend(backend)
        .build()
        .unwrap();

    store.set(["widths", "left", "px"], &800u32).unwrap();
    store.set(["widths", "left", "pct"], &50u32).unwrap();
    store.save_now().unwrap();

    let refused = store
        .kv()
        .map::<String, u32>("widths")
        .expect_err("the map cannot be read over keys that are not its entries");

    let rendered = format!("{refused:?}");
    assert!(
        rendered.contains("widths.left.p"),
        "the report names the key that is not an entry: {rendered}"
    );
    assert!(
        rendered.contains("a map owns the level below it"),
        "and says why it is not one: {rendered}"
    );
}
