//! What a failure actually says, pinned whole.
//!
//! A report is a tree - a chain of contexts with attachments hanging off each
//! frame - and asserting on the variant alone leaves most of it unchecked. The
//! defect these exist to catch is a report that is correctly typed and useless
//! to read: the right `StorageError`, and nothing saying which path, which
//! file, which entry. That has already happened twice.
//!
//! Snapshots rather than assertions because the thing under test is the shape.
//! Adding a frame, dropping an attachment or reordering the chain all change
//! the snapshot, which is the point.

use amethystate::Field;
use amethystate::store::StoreBackend;
use amethystate::store::builder::{StoreBuilder, default_backend};
use amethystate::store::reactive_map_with_path_only;
use amethystate_core::path::StorePath;
use amethystate_core::test_utils::TempPath;
use std::collections::HashMap;
use uuid::Uuid;

mod common;
use common::{per_engine, shape};

#[amethystate::amethystate(prefix = "panel")]
pub struct Panel {
    #[amestate(default = 800u32)]
    pub width: u32,
}

fn store(name: &str) -> (TempPath, amethystate::Store) {
    let path = TempPath::new(name);
    let store = StoreBuilder::new(path.path()).build().unwrap();
    (path, store)
}

/// A level with no name, refused before anything is written.
#[test]
fn a_path_with_an_empty_level() {
    let (_dir, store) = store("report_empty_level");

    let err = store.set(["ui", ""], &1u32).unwrap_err();

    insta::assert_snapshot!("empty_level", shape(&err));
}

/// Bytes that are not the type asked for. The report has to name the type it
/// was asked for and what the codec made of the bytes.
///
/// Named per engine because the codec's own sentence is the engine's, and
/// pinning it is how a difference between them - the text engines wrap this in
/// a `Read` where the flat ones do not - becomes visible rather than folklore.
#[test]
fn bytes_that_are_not_the_type() {
    let (_dir, store) = store("report_decode");
    store.set(["port"], &"not a number".to_string()).unwrap();

    let raw = StoreBackend::get_raw(&store, &StorePath::from_segments(["port"]))
        .unwrap()
        .unwrap();
    let err = store.decode::<u16>(&raw).unwrap_err();

    insta::assert_snapshot!(
        per_engine(default_backend(), "decode_wrong_type"),
        shape(&err)
    );
}

/// Reading a stored value as the wrong type, through the surface a caller
/// actually uses.
///
/// The chain every engine has to give: the operation the caller asked for on
/// the outside, the codec's refusal underneath, and the path in between.
#[test]
fn a_stored_value_read_as_the_wrong_type() {
    let (_dir, store) = store("report_get_wrong_type");
    store
        .set(["cfg", "name"], &"not a number".to_string())
        .unwrap();

    let err = store.get::<u32>(["cfg", "name"]).unwrap_err();

    insta::assert_snapshot!(per_engine(default_backend(), "get_wrong_type"), shape(&err));
}

/// A map entry that will not read. The report has to name the map and the
/// entry, not just the codec's complaint - this is the one that was wrong.
#[test]
fn a_map_entry_that_will_not_read() {
    let (_dir, store) = store("report_map_entry");
    store.set(["cols", "cpu"], &"wide".to_string()).unwrap();
    store.save_now().unwrap();

    let err = reactive_map_with_path_only::<String, u32>(
        &store,
        ["cols"],
        HashMap::new(),
        Uuid::new_v4(),
    )
    .unwrap_err();

    insta::assert_snapshot!(
        per_engine(default_backend(), "map_entry_wrong_type"),
        shape(&err)
    );
}

/// A map default whose key cannot be a level.
#[test]
fn a_map_default_with_an_empty_key() {
    let (_dir, store) = store("report_empty_default");

    let err = reactive_map_with_path_only::<String, u32>(
        &store,
        ["sizes"],
        HashMap::from([(String::new(), 1u32)]),
        Uuid::new_v4(),
    )
    .unwrap_err();

    insta::assert_snapshot!("map_empty_default_key", shape(&err));
}

/// A key that cannot be a level. The map is not at fault and neither is the
/// store, so the report has to name both halves - which map, which key - or
/// there is nothing to go on.
#[test]
fn a_map_key_that_cannot_be_a_level() {
    let (_dir, store) = store("report_map_key");
    let widths = store.kv().map::<String, u64>("cols").unwrap();

    let err = widths.insert(String::new(), &1).unwrap_err();

    insta::assert_snapshot!("map_empty_key", shape(&err));
}

/// The strict write on a key that is not there. `insert` is the one that adds
/// a key, and the refusal has to say which map it was looking in.
#[test]
fn a_strict_write_to_an_absent_key() {
    let (_dir, store) = store("report_map_absent");
    let widths = store.kv().map::<String, u64>("cols").unwrap();

    let err = widths.update("cpu", &1).unwrap_err();

    insta::assert_snapshot!("map_update_on_an_absent_key", shape(&err));
}

/// Both read-modify-write forms refuse an absent key the same way, and each
/// builds its own report rather than going through the other.
#[test]
fn a_read_modify_write_on_an_absent_key() {
    let (_dir, store) = store("report_map_rmw_absent");
    let widths = store.kv().map::<String, u64>("cols").unwrap();

    let refused = [
        widths
            .update_with("cpu", |w| w + 1)
            .map(|_| ())
            .unwrap_err(),
        widths.modify("cpu", |w| *w += 1).unwrap_err(),
    ];

    for (way, err) in ["update_with", "modify"].into_iter().zip(refused) {
        insta::assert_snapshot!(format!("map_{way}_on_an_absent_key"), shape(&err));
    }
}

/// An interceptor turning a write down. The interceptor said nothing beyond
/// no, so the map is all the report can add - and it has to.
#[test]
fn a_write_an_interceptor_turned_down() {
    let (_dir, store) = store("report_map_intercepted");
    let widths = store.kv().map::<String, u64>("cols").unwrap();
    let _guard = widths.intercept(|_| None);

    let err = widths.insert("cpu".to_string(), &1).unwrap_err();

    insta::assert_snapshot!("map_insert_an_interceptor_refused", shape(&err));
}

/// The same refusal aimed at the whole map rather than one key, which is the
/// change that carries no key at all.
#[test]
fn a_clear_an_interceptor_turned_down() {
    let (_dir, store) = store("report_map_clear_intercepted");
    let widths = store.kv().map::<String, u64>("cols").unwrap();
    widths.insert("cpu".to_string(), &1).unwrap();

    let _guard = widths.intercept(|_| None);
    let err = widths.clear().unwrap_err();

    insta::assert_snapshot!("map_clear_an_interceptor_refused", shape(&err));
}

/// A field write an interceptor turned down.
#[test]
fn a_field_write_an_interceptor_turned_down() {
    let (_dir, store) = store("report_field_intercepted");
    let panel = Panel::new_with(&store).unwrap();
    let width = panel.width();
    let _guard = width.intercept(|_| None);

    let err = width.set(9).unwrap_err();

    insta::assert_snapshot!("field_an_interceptor_refused", shape(&err));
}

/// The same on a volatile field, which has no store behind it. The refusal is
/// the whole of what happens, so the report is the only thing the caller gets.
#[test]
fn a_volatile_field_write_an_interceptor_turned_down() {
    let session: Field<String> = Field::new_volatile(
        StorePath::from_segments(["app", "session"]),
        "anonymous".to_string(),
    );
    let _guard = session.intercept(|_| None);

    let err = session.set("alice".to_string()).unwrap_err();

    insta::assert_snapshot!("volatile_field_an_interceptor_refused", shape(&err));
}

/// A `Kv` write at a name that cannot be a level.
#[test]
fn a_kv_name_that_cannot_be_a_level() {
    let (_dir, store) = store("report_kv_name");

    let err = store.kv().set("", &1u32).unwrap_err();

    insta::assert_snapshot!("kv_empty_name", shape(&err));
}

/// A `Kv` write into a namespace, so the report carries both halves.
#[test]
fn a_kv_name_under_a_namespace() {
    let (_dir, store) = store("report_kv_namespace");

    let err = store.kv().namespace("ui").set("", &1u32).unwrap_err();

    insta::assert_snapshot!("kv_empty_name_in_namespace", shape(&err));
}
