#![cfg(feature = "json")]

use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate::{AmeData, amethystate, migrate};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

mod v1 {
    use super::*;

    #[amethystate(prefix = "app", version = 1)]
    pub struct Settings {
        #[amestate(default = "localhost".to_string())]
        pub host: String,
    }
}

#[amethystate(prefix = "app", version = 2)]
pub struct Settings {
    #[amestate(default = "localhost".to_string())]
    pub host: String,

    #[amestate(default = 8080u16)]
    pub port: u16,
}

#[migrate]
fn settings_v1_to_v2(
    old: AmeData<v1::Settings>,
) -> amethystate::MigrationResult<AmeData<Settings>> {
    Ok(AmeData::<Settings> {
        host: old.host,
        port: 8080,
    })
}

fn meta_path(at: &Path) -> PathBuf {
    at.with_extension("meta")
}

fn settle() {
    std::thread::sleep(std::time::Duration::from_millis(120));
}

fn seeded(suffix: &str) -> TempPath {
    let path = TempPath::new(suffix);

    {
        let store = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build()
            .unwrap();
        let settings = v1::Settings::new_with(&store).unwrap();
        settings.host().set("10.0.0.1".to_string()).unwrap();
        drop(settings);
        store.save_now().unwrap();
    }
    settle();

    {
        let (store, report) = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build_with_migration()
            .unwrap();
        assert!(!report.has_failures());
        let _settings = Settings::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    settle();

    path
}

#[test]
fn a_migrated_prefix_records_the_shape_it_arrived_at() {
    let path = seeded("drift_records_the_arrival");
    let meta = std::fs::read_to_string(meta_path(path.path())).unwrap();

    let held: serde_json::Value = serde_json::from_str(&meta).unwrap();
    let snapshots = held["schema.app"].as_array().unwrap();

    let at_v2 = snapshots
        .iter()
        .find(|one| one["version"] == 2)
        .unwrap_or_else(|| panic!("nothing at `app` records version 2:\n{meta}"));

    let named: Vec<&str> = at_v2["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["name"].as_str().unwrap())
        .collect();

    assert_eq!(named, ["host", "port"], "{meta}");
}

fn a_build_that_declared(path: &TempPath, fields: serde_json::Value) {
    let at = meta_path(path.path());
    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();

    let snapshots = held["schema.app"].as_array_mut().unwrap();
    let at_v2 = snapshots
        .iter_mut()
        .find(|one| one["version"] == 2)
        .expect("nothing at `app` records version 2");

    at_v2["fields"] = fields;

    std::fs::write(&at, serde_json::to_string_pretty(&held).unwrap()).unwrap();
}

fn field(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "type_name": "String",
        "shape": { "role": "field", "optional": false },
    })
}

fn a_build_that_also_declared(path: &TempPath, name: &str) {
    a_build_that_declared(
        path,
        serde_json::json!([field("host"), field("port"), field(name)]),
    );
}

fn opened(path: &TempPath) -> amethystate::MigrationReport {
    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build_with_migration()
        .unwrap();
    drop(store);
    settle();
    report
}

#[test]
fn a_place_the_code_stopped_declaring_is_drift() {
    let path = seeded("drift_a_place_released");
    a_build_that_also_declared(&path, "token");

    let report = opened(&path);

    assert!(report.has_drift(), "a released place was not reported");

    let nagging: Vec<_> = report
        .components
        .iter()
        .flat_map(|component| component.nagging.iter())
        .collect();

    assert_eq!(nagging.len(), 1);
    assert_eq!(nagging[0].prefix.to_string(), "app");

    let said: Vec<String> = nagging[0]
        .moved
        .iter()
        .map(|one| format!("{:?} {one}", one.verdict()))
        .collect();

    assert_eq!(
        said,
        ["Breaks `token` was declared before and is not now"],
        "a move is named under the prefix, and the prefix is on the record"
    );

    let removed: Vec<&str> = nagging[0]
        .diff
        .as_ref()
        .expect("a break with no diff beside it")
        .removed
        .iter()
        .map(|field| field.name.to_string())
        .map(|name| Box::leak(name.into_boxed_str()) as &str)
        .collect();

    assert_eq!(removed, ["token"]);
}

#[cfg(feature = "diagnostics")]
#[test]
fn the_drift_a_person_reads() {
    let path = seeded("drift_rendered");

    a_build_that_declared(
        &path,
        serde_json::json!([
            field("token"),
            {
                "name": "port",
                "type_name": "HashMap<String, u16>",
                "shape": { "role": "map", "optional": false },
            },
            {
                "name": "host",
                "type_name": "String",
                "shape": { "role": "field", "optional": true },
            },
        ]),
    );

    let report = opened(&path);
    let drift = report.drift();

    assert_eq!(drift.len(), 1);

    let shown = format!("{:?}", drift[0]);
    println!("{shown}");

    assert!(shown.contains("app.token"), "{shown}");
    assert!(shown.contains("amethystate::drift"), "{shown}");
}

#[test]
fn drift_nobody_answered_is_reported_again_on_the_next_open() {
    let path = seeded("drift_said_twice");
    a_build_that_also_declared(&path, "token");

    assert!(opened(&path).has_drift(), "the first open said nothing");
    assert!(
        opened(&path).has_drift(),
        "an open that only reported drift recorded the shape anyway, so the \
         second open had nothing left to compare against"
    );
}
