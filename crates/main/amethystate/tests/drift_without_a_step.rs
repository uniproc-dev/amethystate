#![cfg(feature = "json")]

use amethystate::amethystate;
use amethystate::store::builder::{Backend, StoreBuilder};
use amethystate_core::test_utils::TempPath;
use std::path::{Path, PathBuf};

#[amethystate(prefix = "solo", version = 1)]
pub struct Solo {
    #[amestate(default = "localhost".to_string())]
    pub host: String,
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
        let (store, report) = StoreBuilder::new(path.path())
            .backend(Backend::Json)
            .build_with_migration()
            .unwrap();
        assert!(!report.has_failures());
        let _solo = Solo::new_with(&store).unwrap();
        store.save_now().unwrap();
    }
    settle();

    let at = meta_path(path.path());
    let mut held: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&at).unwrap()).unwrap();

    held["schema.solo"] = serde_json::json!([{
        "version": 1,
        "struct_name": "Solo",
        "fields": [
            {
                "name": "host",
                "type_name": "String",
                "shape": { "role": "field", "optional": false },
            },
            {
                "name": "token",
                "type_name": "String",
                "shape": { "role": "field", "optional": false },
            },
        ],
    }]);

    std::fs::write(&at, serde_json::to_string_pretty(&held).unwrap()).unwrap();

    path
}

#[test]
fn a_binary_with_no_migration_step_still_has_its_drift_reported() {
    let path = seeded("drift_without_a_step");

    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build_with_migration()
        .unwrap();
    drop(store);

    assert!(
        report.has_drift(),
        "nothing declared a step at `solo`, so nothing looked at it"
    );

    let nagging: Vec<_> = report
        .components
        .iter()
        .flat_map(|component| component.nagging.iter())
        .collect();

    assert_eq!(nagging.len(), 1);
    assert_eq!(nagging[0].prefix.to_string(), "solo");
    assert_eq!(
        nagging[0]
            .moved
            .iter()
            .map(|one| one.to_string())
            .collect::<Vec<_>>(),
        ["`token` was declared before and is not now"]
    );
}

#[test]
fn a_plain_open_does_not_record_over_the_drift_it_found() {
    let path = seeded("drift_without_the_pass");

    let store = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build()
        .unwrap();
    drop(store);
    settle();

    let (store, report) = StoreBuilder::new(path.path())
        .backend(Backend::Json)
        .build_with_migration()
        .unwrap();
    drop(store);

    assert!(
        report.has_drift(),
        "`build` hands no report back, so what it saw is only visible here: \
         an open that missed the drift would have recorded the declaration \
         over it and left this one nothing to compare against"
    );
}
