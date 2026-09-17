#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, amethystate, migrate};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

mod v1 {
    use super::*;

    #[amethystate(prefix = "kept", version = 1)]
    pub struct Keep {
        #[amestate(default = 1u32)]
        pub held: u32,
    }
}

#[amethystate(prefix = "kept", version = 2)]
pub struct Keep {
    #[amestate(default = 1u32)]
    pub held: u32,
}

#[migrate]
fn keep_v1_to_v2(old: AmeData<v1::Keep>) -> amethystate::MigrationResult<AmeData<Keep>> {
    Ok(AmeData::<Keep> { held: old.held })
}

fn meta_path(at: &Path) -> PathBuf {
    at.with_extension("meta")
}

fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(120));
}

/// A record for a struct this binary does not declare, as a build that has been
/// deleted would have left it.
fn a_struct_that_is_gone(path: &TempPath) {
    let at = meta_path(path.path());
    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();

    held["schema.kept"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "version": 1,
            "id": "ghost",
            "struct_name": "Ghost",
            "fields": [
                { "name": "gone", "type_name": "String",
                  "shape": { "role": "field", "optional": false } },
            ],
        }));

    std::fs::write(&at, serde_json::to_string_pretty(&held).unwrap()).unwrap();
}

fn names(path: &TempPath) -> Vec<String> {
    let meta = std::fs::read_to_string(meta_path(path.path())).unwrap();
    let held: serde_json::Value = serde_json::from_str(&meta).unwrap();

    held["schema.kept"]
        .as_array()
        .map(|records| {
            records
                .iter()
                .filter_map(|one| one["struct_name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_step_that_moves_the_prefix_on_retires_what_it_left_behind() {
    let path = TempPath::new("record_left_behind");

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let _early = v1::Keep::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    settle();

    a_struct_that_is_gone(&path);

    assert!(
        names(&path).contains(&"Ghost".to_string()),
        "the fixture did not take"
    );

    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .migrate()
        .unwrap();
    drop(store);
    settle();

    assert!(
        !report.has_drift(),
        "a step took the prefix past the version that record stands at, which is \
         what answers for the places it held"
    );
    assert_eq!(
        names(&path),
        ["Keep"],
        "the record the migration answered for is still there to be reported again"
    );

    let (store, again) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .migrate()
        .unwrap();
    drop(store);

    assert!(!again.has_drift(), "it came back on the next open");
}

#[test]
fn a_record_the_prefix_has_not_moved_past_is_left_where_it_is() {
    let path = TempPath::new("record_not_answered");

    {
        let (store, _) = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .migrate()
            .unwrap();
        let _now = Keep::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let at = meta_path(path.path());
    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();

    held["schema.kept"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "version": 2,
            "id": "ghost",
            "struct_name": "Ghost",
            "fields": [
                { "name": "gone", "type_name": "String",
                  "shape": { "role": "field", "optional": false } },
            ],
        }));

    std::fs::write(&at, serde_json::to_string_pretty(&held).unwrap()).unwrap();

    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .migrate()
        .unwrap();
    drop(store);

    assert!(
        report.has_drift(),
        "nothing has answered for `gone`, and the prefix stands where that \
         record does"
    );
}
