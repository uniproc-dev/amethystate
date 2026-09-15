#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

#[amethystate(prefix = "shared", version = 1)]
pub struct Widths {
    #[amestate(default = 200u32)]
    pub left: u32,
}

#[amethystate(prefix = "shared", id = "theme", version = 2)]
pub struct Theme {
    #[amestate(default = "dark".to_string())]
    pub name: String,
}

fn meta_path(at: &Path) -> PathBuf {
    at.with_extension("meta")
}

fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(120));
}

fn recorded(path: &TempPath) -> Vec<String> {
    let meta = std::fs::read_to_string(meta_path(path.path())).unwrap();
    let held: serde_json::Value = serde_json::from_str(&meta).unwrap();

    held["schema.shared"]
        .as_array()
        .map(|records| {
            records
                .iter()
                .filter_map(|one| one["struct_name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn seeded(suffix: &str) -> TempPath {
    let path = TempPath::new(suffix);

    {
        let (store, _) = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build_with_migration()
            .unwrap();
        let _widths = Widths::new_with(&store).unwrap();
        let _theme = Theme::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    settle();

    path
}

#[test]
fn the_lower_versioned_declaration_at_a_prefix_is_looked_at_too() {
    let path = seeded("drift_lower_version");

    let at = meta_path(path.path());
    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();

    let records = held["schema.shared"].as_array_mut().unwrap();
    let widths = records
        .iter_mut()
        .find(|one| one["struct_name"] == "Widths")
        .expect("`Widths` was never recorded");

    widths["fields"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "name": "right",
            "type_name": "u32",
            "shape": { "role": "field", "optional": false },
        }));

    std::fs::write(&at, serde_json::to_string_pretty(&held).unwrap()).unwrap();

    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build_with_migration()
        .unwrap();
    drop(store);

    assert!(
        report.has_drift(),
        "`Widths` released a place and nothing said so, because `Theme` is \
         declared at the same prefix at a higher version"
    );
}

#[test]
fn a_neighbour_is_still_recorded_while_the_one_beside_it_drifts() {
    let path = TempPath::new("drift_one_declaration");

    {
        let (store, _) = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build_with_migration()
            .unwrap();
        let _widths = Widths::new_with(&store).unwrap();
        let _theme = Theme::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let at = meta_path(path.path());
    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();

    held["schema.shared"] = serde_json::json!([{
        "version": 2,
        "id": "theme",
        "struct_name": "Theme",
        "fields": [
            { "name": "name", "type_name": "String",
              "shape": { "role": "field", "optional": false } },
            { "name": "token", "type_name": "String",
              "shape": { "role": "field", "optional": false } },
        ],
    }]);

    std::fs::write(&at, serde_json::to_string_pretty(&held).unwrap()).unwrap();

    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build_with_migration()
        .unwrap();
    drop(store);
    settle();

    assert!(report.has_drift(), "the tampered record was not reported");

    let mut names = recorded(&path);
    names.sort();

    assert_eq!(
        names,
        ["Theme", "Widths"],
        "a declaration was held back because a different one at the same prefix \
         drifted, and it has no way to make that stop"
    );
}
