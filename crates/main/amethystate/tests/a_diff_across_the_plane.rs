#![cfg(feature = "json")]

use amethystate::store::backend::text::document::TextDocument;
use amethystate::store::backend::text::json::json_doc::JsonDocument;
use amethystate::store::backend::text::json::json_tree::JsonTree;
use amethystate::store::backend::text::store::diff_documents;

fn between<D: TextDocument>(held: &str, there: &str) -> usize {
    diff_documents::<D>(
        &D::parse(held).expect("the first reading parses"),
        &D::parse(there).expect("the second reading parses"),
        1,
    )
    .expect("the diff runs")
    .len()
}

#[test]
fn a_plane_key_whose_name_spells_a_path_reports_its_change() {
    let held = r#"{ "plugin0.width": 1, "plugin1.width": 2 }"#;
    let there = r#"{ "plugin0.width": 9, "plugin1.width": 2 }"#;

    assert_eq!(between::<JsonTree>(held, there), 1);
    assert_eq!(between::<JsonDocument>(held, there), 1);
}

#[test]
fn a_value_under_a_plane_level_reports_its_change() {
    let held = r#"{ "plugin0": { "width": 1, "height": 2 } }"#;
    let there = r#"{ "plugin0": { "width": 9, "height": 2 } }"#;

    assert_eq!(between::<JsonTree>(held, there), 1);
    assert_eq!(between::<JsonDocument>(held, there), 1);
}

#[test]
fn a_plane_key_added_and_one_removed_are_both_reported() {
    let held = r#"{ "a.b": 1, "c.d": 2 }"#;
    let there = r#"{ "a.b": 1, "e.f": 3 }"#;

    assert_eq!(between::<JsonTree>(held, there), 2);
    assert_eq!(between::<JsonDocument>(held, there), 2);
}
