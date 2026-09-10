#![cfg(feature = "json")]

use amethystate::store::backend::text::document::TextDocument;
use amethystate::store::backend::text::json::json_doc::JsonDocument;
use amethystate::store::backend::text::json::json_tree::JsonTree;
use amethystate::store::backend::text::store::diff_documents;
use amethystate::{ReactiveMap, StoreEvent};

#[amethystate::amethystate(prefix = "panels")]
pub struct Panels {
    #[amestate(default = 0)]
    pub width: u32,

    #[amestate(default = {})]
    pub items: ReactiveMap<String, u64>,
}

fn spelled(events: &[StoreEvent]) -> Vec<String> {
    let mut said: Vec<String> = events
        .iter()
        .map(|event| {
            format!(
                "{} {:?} {:?} -> {:?}",
                event.path,
                event.op,
                event.old.as_deref().map(String::from_utf8_lossy),
                event.new.as_deref().map(String::from_utf8_lossy),
            )
        })
        .collect();
    said.sort();
    said
}

fn between(held: &str, there: &str) -> Vec<String> {
    let by_reference = diff_documents::<JsonDocument>(
        &JsonDocument::parse(held).expect("the first reading parses"),
        &JsonDocument::parse(there).expect("the second reading parses"),
        1,
    )
    .expect("the reference diffs");

    let by_tree = diff_documents::<JsonTree>(
        &JsonTree::parse(held).expect("the first reading parses"),
        &JsonTree::parse(there).expect("the second reading parses"),
        1,
    )
    .expect("the owned tree diffs");

    assert_eq!(
        spelled(&by_tree),
        spelled(&by_reference),
        "the two engines disagree about this edit:\n{held}\n{there}"
    );

    spelled(&by_reference)
}

const HELD: &str = r#"{
    "panels": {
        "width": 1280,
        "items": { "cpu": 1, "mem": 2, "gpu": 3 }
    }
}"#;

#[test]
fn a_value_deep_in_the_tree_is_reported() {
    let said = between(
        HELD,
        r#"{ "panels": { "width": 1280, "items": { "cpu": 9, "mem": 2, "gpu": 3 } } }"#,
    );

    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("panels.items.cpu "), "{said:?}");
}

#[test]
fn a_field_beside_the_map_is_reported() {
    let said = between(
        HELD,
        r#"{ "panels": { "width": 800, "items": { "cpu": 1, "mem": 2, "gpu": 3 } } }"#,
    );

    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("panels.width "), "{said:?}");
}

#[test]
fn an_entry_added_and_one_removed_are_both_reported() {
    let said = between(
        HELD,
        r#"{ "panels": { "width": 1280, "items": { "cpu": 1, "mem": 2, "net": 4 } } }"#,
    );

    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said.iter().any(|s| s.starts_with("panels.items.gpu ")));
    assert!(said.iter().any(|s| s.starts_with("panels.items.net ")));
}

#[test]
fn nothing_changed_is_nothing_said() {
    assert!(between(HELD, HELD).is_empty());
}

#[test]
fn a_map_emptied_by_hand_is_reported() {
    let said = between(HELD, r#"{ "panels": { "width": 1280, "items": {} } }"#);

    assert!(
        said.iter().any(|s| s.starts_with("panels.items.cpu ")
            || s.starts_with("panels.items ")),
        "{said:?}"
    );
    assert!(!said.iter().any(|s| s.starts_with("panels.width ")), "{said:?}");
}

#[test]
fn a_declared_prefix_overwritten_with_a_scalar_is_reported() {
    assert!(!between(HELD, r#"{ "panels": 7 }"#).is_empty());
}

#[test]
fn a_level_whose_children_have_no_paths_is_reported_as_itself() {
    let said = between(
        r#"{ "panels": { "width": 1280, "items": { "": 1 } } }"#,
        r#"{ "panels": { "width": 1280, "items": { "": 2 } } }"#,
    );

    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("panels.items "), "{said:?}");
}

#[test]
fn a_child_with_no_path_does_not_silence_its_siblings() {
    let said = between(
        r#"{ "panels": { "width": 1280, "items": { "": 1, "cpu": 1 } } }"#,
        r#"{ "panels": { "width": 1280, "items": { "": 1, "cpu": 9 } } }"#,
    );

    assert!(said.iter().any(|s| s.starts_with("panels.items.cpu ")), "{said:?}");
}

#[test]
fn a_level_whose_entries_were_reordered_holds_the_same_values() {
    let said = between(
        HELD,
        r#"{ "panels": { "width": 1280, "items": { "gpu": 3, "cpu": 1, "mem": 2 } } }"#,
    );

    assert!(said.is_empty(), "{said:?}");
}

#[test]
fn a_name_inserted_in_the_middle_does_not_shift_the_rest() {
    let said = between(
        HELD,
        r#"{ "panels": { "width": 1280, "items": { "cpu": 1, "disk": 8, "mem": 2, "gpu": 3 } } }"#,
    );

    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].starts_with("panels.items.disk "), "{said:?}");
}

#[test]
fn a_level_that_moved_wholesale_reports_every_entry_once() {
    let said = between(
        HELD,
        r#"{ "panels": { "width": 1280, "items": { "a": 1, "b": 2, "c": 3 } } }"#,
    );

    let mut names: Vec<&String> = said.iter().collect();
    names.dedup();
    assert_eq!(names.len(), said.len(), "a path was reported twice: {said:?}");
    assert_eq!(said.len(), 6, "{said:?}");
}
