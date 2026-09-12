#![cfg(feature = "json")]

use amethystate::store::StorePath;
use amethystate::store::backend::text::document::TextDocument;
use amethystate::store::backend::text::json::json_doc::JsonDocument;
use amethystate::store::backend::text::json::json_tree::JsonTree;
use amethystate::store::backend::text::store::diff_documents;

const SRC: &str = r#"{
    "ui.theme": "dark",
    "ui.width": 1280,
    "limits": { "depth": 4, "keys": null, "ratio": 0.5 },
    "flags": ["open", "pinned"],
    "deep": { "a": { "b": { "c": true } } },
    "signed": -17,
    "huge": 18446744073709551615,
    "name\\.with\\.dots": "one level"
}"#;

const EDITED: &str = r#"{
    "ui.theme": "light",
    "ui.width": 1280,
    "limits": { "depth": 4, "keys": null, "ratio": 0.5 },
    "flags": ["open", "pinned", "resizable"],
    "deep": { "a": { "b": { "c": true } } },
    "huge": 18446744073709551615,
    "added": 1,
    "name\\.with\\.dots": "one level"
}"#;

fn spelled(events: &[amethystate::store::StoreEvent]) -> Vec<String> {
    events
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
        .collect()
}

fn paths<D: TextDocument>(doc: &D) -> Vec<String> {
    doc.scan(&StorePath::root())
        .expect("the root scans")
        .into_iter()
        .map(|(at, _)| at.to_string())
        .collect()
}

fn bytes_at<D: TextDocument>(doc: &D, at: &StorePath) -> Option<String> {
    let node = doc.get(at)?;
    let bytes = D::node_to_bytes(node).expect("a node renders");
    Some(String::from_utf8(bytes).expect("json is utf-8"))
}

#[test]
fn it_renders_what_the_reference_renders() {
    let held = JsonDocument::parse(SRC).unwrap();
    let owned = JsonTree::parse(SRC).unwrap();

    assert_eq!(owned.serialize().unwrap(), held.serialize().unwrap());
}

#[test]
fn it_lists_the_same_keys_in_the_same_order() {
    let held = JsonDocument::parse(SRC).unwrap();
    let owned = JsonTree::parse(SRC).unwrap();

    assert_eq!(paths(&owned), paths(&held));
}

#[test]
fn it_answers_every_path_with_the_same_bytes() {
    let held = JsonDocument::parse(SRC).unwrap();
    let owned = JsonTree::parse(SRC).unwrap();

    for at in held.scan(&StorePath::root()).unwrap() {
        let at = at.0;
        assert_eq!(
            bytes_at(&owned, &at),
            bytes_at(&held, &at),
            "the two documents disagree at {at}"
        );
    }

    let deep = StorePath::from_segments(["deep", "a", "b", "c"]);
    assert_eq!(bytes_at(&owned, &deep), Some("true".to_string()));
    assert_eq!(bytes_at(&owned, &deep), bytes_at(&held, &deep));
}

#[test]
fn it_writes_and_deletes_the_way_the_reference_does() {
    let mut held = JsonDocument::parse(SRC).unwrap();
    let mut owned = JsonTree::parse(SRC).unwrap();

    let under = StorePath::from_segments(["limits", "depth"]);
    held.set(&under, JsonDocument::bytes_to_node(b"9").unwrap())
        .unwrap();
    owned
        .set(&under, JsonTree::bytes_to_node(b"9").unwrap())
        .unwrap();

    let fresh = StorePath::from_segments(["limits", "added", "leaf"]);
    held.set(&fresh, JsonDocument::bytes_to_node(b"\"new\"").unwrap())
        .unwrap();
    owned
        .set(&fresh, JsonTree::bytes_to_node(b"\"new\"").unwrap())
        .unwrap();

    held.delete(&StorePath::segment("signed")).unwrap();
    owned.delete(&StorePath::segment("signed")).unwrap();

    held.delete_subtree(&StorePath::segment("deep")).unwrap();
    owned.delete_subtree(&StorePath::segment("deep")).unwrap();

    assert_eq!(owned.serialize().unwrap(), held.serialize().unwrap());
    assert_eq!(paths(&owned), paths(&held));
}

#[test]
fn it_reports_the_same_changes_between_two_readings() {
    let held = JsonDocument::parse(SRC).unwrap();
    let held_after = JsonDocument::parse(EDITED).unwrap();
    let owned = JsonTree::parse(SRC).unwrap();
    let owned_after = JsonTree::parse(EDITED).unwrap();

    let by_reference = diff_documents::<JsonDocument>(&held, &held_after, 1).unwrap();
    let by_tree = diff_documents::<JsonTree>(&owned, &owned_after, 1).unwrap();

    assert!(
        !by_reference.is_empty(),
        "the two readings differ, so there is something to report"
    );
    assert_eq!(spelled(&by_tree), spelled(&by_reference));
}

#[test]
fn a_write_leaves_a_copy_taken_before_it_alone() {
    let owned = JsonTree::parse(SRC).unwrap();
    let before = owned.clone();

    let mut after = owned;
    after
        .set(
            &StorePath::from_segments(["limits", "depth"]),
            JsonTree::bytes_to_node(b"9").unwrap(),
        )
        .unwrap();

    let at = StorePath::from_segments(["limits", "depth"]);
    assert_eq!(bytes_at(&before, &at), Some("4".to_string()));
    assert_eq!(bytes_at(&after, &at), Some("9".to_string()));
}

const PLANE_AND_TREE: &str = r#"{
    "ui.theme": "dark",
    "ui.width": 1280,
    "name\\.with\\.dots": "one level",
    "deep": { "a": { "b": 1, "c": 2 }, "d": 3 },
    "kept": { "only": true }
}"#;

fn changed(src: &str, edits: &[(&str, Option<serde_json::Value>)]) -> String {
    let mut held: serde_json::Value = serde_json::from_str(src).unwrap();
    let map = held.as_object_mut().unwrap();

    for (key, value) in edits {
        match value {
            Some(value) => {
                map.insert((*key).to_string(), value.clone());
            }
            None => {
                map.remove(*key);
            }
        }
    }

    serde_json::to_string(&held).unwrap()
}

#[test]
fn the_changes_reported_do_not_depend_on_how_much_of_the_document_was_read() {
    let cases: Vec<String> = vec![
        changed(PLANE_AND_TREE, &[]),
        changed(PLANE_AND_TREE, &[("ui.theme", Some("light".into()))]),
        changed(PLANE_AND_TREE, &[("ui.theme", None)]),
        changed(PLANE_AND_TREE, &[("added", Some(7.into()))]),
        changed(
            PLANE_AND_TREE,
            &[("deep", Some(serde_json::json!({ "a": { "b": 9 } })))],
        ),
        changed(PLANE_AND_TREE, &[("deep", None)]),
        changed(PLANE_AND_TREE, &[("kept", Some(serde_json::json!({})))]),
        changed(
            PLANE_AND_TREE,
            &[("name\\.with\\.dots", Some("moved".into()))],
        ),
        "{}".to_string(),
    ];

    for after in cases {
        let held = JsonDocument::parse(PLANE_AND_TREE).unwrap();
        let held_after = JsonDocument::parse(&after).unwrap();
        let owned = JsonTree::parse(PLANE_AND_TREE).unwrap();
        let owned_after = JsonTree::parse(&after).unwrap();

        assert_eq!(
            spelled(&diff_documents::<JsonTree>(&owned, &owned_after, 1).unwrap()),
            spelled(&diff_documents::<JsonDocument>(&held, &held_after, 1).unwrap()),
            "the two documents disagree about this edit:\n{after}"
        );
    }
}

#[test]
fn reordering_a_level_is_a_change() {
    let held = r#"{ "cfg": { "a": 1, "b": 2 } }"#;
    let moved = r#"{ "cfg": { "b": 2, "a": 1 } }"#;

    for (engine, events) in [
        (
            "reference",
            diff_documents::<JsonDocument>(
                &JsonDocument::parse(held).unwrap(),
                &JsonDocument::parse(moved).unwrap(),
                1,
            )
            .unwrap(),
        ),
        (
            "owned tree",
            diff_documents::<JsonTree>(
                &JsonTree::parse(held).unwrap(),
                &JsonTree::parse(moved).unwrap(),
                1,
            )
            .unwrap(),
        ),
    ] {
        assert!(
            !events.is_empty(),
            "{engine} called a level whose order changed no change, and the two render \
             differently"
        );
    }
}

#[test]
fn a_sign_on_a_zero_is_a_change() {
    let held = r#"{ "gain": -0.0 }"#;
    let flipped = r#"{ "gain": 0.0 }"#;

    for (engine, events) in [
        (
            "reference",
            diff_documents::<JsonDocument>(
                &JsonDocument::parse(held).unwrap(),
                &JsonDocument::parse(flipped).unwrap(),
                1,
            )
            .unwrap(),
        ),
        (
            "owned tree",
            diff_documents::<JsonTree>(
                &JsonTree::parse(held).unwrap(),
                &JsonTree::parse(flipped).unwrap(),
                1,
            )
            .unwrap(),
        ),
    ] {
        assert!(
            !events.is_empty(),
            "{engine} called a sign flip no change, and the two render differently"
        );
    }
}

#[test]
fn a_name_no_path_can_hold_does_not_silence_the_rest() {
    let held = r#"{ "": "untouchable", "ui": { "theme": "dark" } }"#;
    let edited = r#"{ "": "untouchable", "ui": { "theme": "light" } }"#;

    let events = diff_documents::<JsonTree>(
        &JsonTree::parse(held).unwrap(),
        &JsonTree::parse(edited).unwrap(),
        1,
    )
    .unwrap();

    assert!(
        spelled(&events).iter().any(|said| said.contains("light")),
        "the edit beside an unaddressable name was not reported: {:?}",
        spelled(&events)
    );
}
